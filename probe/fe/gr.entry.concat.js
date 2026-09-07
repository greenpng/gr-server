 /* greenpng entry | 1.0.2 */

/* ---- gr.fe_impl.js ---- */
/**
 * FE implementation identity — distinct from server product_version.
 * Build injects window.__GR_BUILD_IMPL__ (= root VERSION) into each ship artifact.
 * Runtime records which modules actually loaded so every batch can stamp fe_impl_*.
 *
 * Performance: O(1) memory writes; no network; upload adds a few short strings only.
 */
(function (global) {
  "use strict";
  function buildImpl() {
    try {
      if (global.__GR_BUILD_IMPL__) return String(global.__GR_BUILD_IMPL__);
    } catch (e0) {}
    return "";
  }
  function ensure() {
    try {
      if (!global.__GR_FE_IMPL__ || typeof global.__GR_FE_IMPL__ !== "object") {
        global.__GR_FE_IMPL__ = Object.create(null);
      }
      return global.__GR_FE_IMPL__;
    } catch (e1) {
      return Object.create(null);
    }
  }
  /** Record that module `name` executed from build `ver` (default: this file's build). */
  function noteModule(name, ver) {
    try {
      var m = ensure();
      var v = String(ver || buildImpl() || "");
      if (name) m[String(name)] = v;
      if (v) {
        if (!m.build) m.build = v;
        // Prefer strongest content-proven modules for "fe_impl_version"
        if (name === "lite" || name === "hard" || name === "loader" || name === "entry") {
          if (name === "lite" || name === "hard") m.content = v;
        }
      }
      if (v && !global.__GR_FE_IMPL_VERSION__) {
        global.__GR_FE_IMPL_VERSION__ = v;
      }
      if (name === "lite" && v) {
        global.__GR_FE_LITE_IMPL__ = v;
      }
      if (name === "hard" && v) {
        global.__GR_FE_HARD_IMPL__ = v;
      }
    } catch (e2) {}
  }
  /** Server product epoch (grant / inject) — must match seal allowlist. */
  function serverProduct() {
    try {
      if (global.__GR_SERVER_PRODUCT_VERSION__)
        return String(global.__GR_SERVER_PRODUCT_VERSION__);
    } catch (eS0) {}
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (eS1) {}
    try {
      var b = global.__GR_BOOT__ || {};
      if (b.product_version) return String(b.product_version);
      if (b.version) return String(b.version);
    } catch (eS2) {}
    return "";
  }

  /** Snapshot for upload stamping (shallow). */
  function snapshot() {
    var m = ensure();
    var build = buildImpl() || m.build || "";
    var content = m.content || m.lite || m.hard || "";
    var loader = m.loader || "";
    var entry = m.entry || "";
    var product = serverProduct();
    // Seal gate binds fe_impl_version to product epoch. Prefer server product so
    // sticky/old module BUILD_IMPL stamps (v5.8.*) never reject B10 in lab/prod.
    // Content/build remain diagnostic side fields.
    var effective = product || content || build || "";
    try {
      if (effective) global.__GR_FE_IMPL_VERSION__ = effective;
    } catch (eEff) {}
    return {
      build: build,
      loader: loader,
      entry: entry,
      lite: m.lite || "",
      hard: m.hard || "",
      content: content,
      product: product,
      fe_impl_version: effective,
      fe_loader_impl: loader || build,
      fe_entry_impl: entry || "",
      fe_lite_impl: m.lite || "",
      fe_hard_impl: m.hard || "",
    };
  }
  function markContentProven(ver) {
    try {
      var v = String(ver || buildImpl() || "");
      var m = ensure();
      if (v) {
        m.content = v;
        m.lite = m.lite || v;
        m.hard = m.hard || v;
        global.__GR_FE_IMPL_VERSION__ = v;
        global.__GR_FE_PACKS_VERSION__ = v;
        global.__GR_FE_CODE_VERSION__ = v;
      }
    } catch (e3) {}
  }
  global.GRFeImpl = {
    buildImpl: buildImpl,
    noteModule: noteModule,
    snapshot: snapshot,
    markContentProven: markContentProven,
    ensure: ensure,
  };
  // Self-register if this file carries a build stamp.
  try {
    if (buildImpl()) noteModule("fe_impl_lib", buildImpl());
  } catch (e4) {}
})(typeof window !== "undefined" ? window : globalThis);

/* ---- storage.js ---- */
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

/* ---- probe_lifecycle.js ---- */
/**
 * GR probe lifecycle — thin FE self-consistency (no heavy brain in browser).
 *
 * States: idle_cool | need_probe | probing | retry_backoff | failed_quiet | complete_cool
 * Authority: product_version + server cool/final; FE only gap-fills and backs off.
 *
 * @see reports/FE_PROBE_LIFECYCLE_DESIGN_V5852.md
 */
(function (global) {
  "use strict";

  var STATE = {
    IDLE_COOL: "idle_cool",
    NEED_PROBE: "need_probe",
    PROBING: "probing",
    RETRY_BACKOFF: "retry_backoff",
    FAILED_QUIET: "failed_quiet",
    COMPLETE_COOL: "complete_cool",
  };

  /**
   * Hard anchors: more retries. Soft: fewer then drop pack (not whole cycle).
   * multi_tick_max is a SAFETY CAP only — authority to stop is server
   * stop_probe+coverage / brain_schedule_final / 410 (see FE_BE_STATE_MACHINE_AUTHORITY_V5855).
   */
  var POLICY = {
    /** Hard anchors (B10/B0/B2…): enough retries for seal races + short network blips. */
    hard_max_attempts: 10,
    soft_max_attempts: 3,
    /** Silicon deepen (B10x_*): short-visit front-loaded retries (network blips). */
    deepen_max_attempts: 8,
    /** Continuous RPA (B11): tight cap — never soft_exhausted storms. */
    rpa_max_attempts: 2,
    /**
     * Heavy (B10x/B7) concurrency. Keep 1 under real visitor networks —
     * prod v150 zhanso: concurrent B10x+B47 → Failed to fetch / hard_exhausted mis-tag.
     */
    heavy_max_inflight: 1,
    /** ms delays by attempt index (1-based → index 0) */
    /** Front-load hard (B10) retries for short dwell + visitor network blips. */
    hard_backoff_ms: [200, 500, 1100, 2200, 4500, 9000, 16000, 24000],
    soft_backoff_ms: [2000, 6000, 15000],
    /** Front-load silicon deepen retries for short dwell + visitor network. */
    deepen_backoff_ms: [200, 500, 1200, 2800, 6000, 12000, 20000],
    /** Lane-C must-land (noderiv/rint/ulp) — tighter than generic deepen. */
    commercial_land_backoff_ms: [120, 360, 680, 1100, 1600, 2000],
    rpa_backoff_ms: [3000, 12000],
    /** global fails in rolling window before soft quiet */
    fail_budget_n: 18,
    fail_budget_window_ms: 90000,
    /** After RPA exhaust, quiet only that pack class (ms). */
    rpa_quiet_ms: 90000,
    /**
     * After deepen exhaust quiet — shorter so B10-landed re-kick lands B10x same session.
     */
    deepen_quiet_ms: 22000,
    /**
     * hard SLA re-kick for missing B10 — front-load for short visits (p50 dwell ~2–4s).
     * Extended tail so multi-tab / background multipath can still re-arm without refresh.
     * Long-window collect is also owned by GRProbeSelfHeal Loop C.
     */
    hard_sla_delays_ms: [800, 2200, 5500, 12000, 20000, 45000, 90000],
    /**
     * iss/70: multi_tick_max is abnormal safety only.
     * Normal path stops earlier via plan_epoch / empty_kick / server terminal.
     */
    multi_tick_max: 16,
    /** Wait ticks with 0 new kicks before giving up. */
    empty_kick_patience: 5,
    /** After B10 lands, even fewer empty plan ticks before stop. */
    empty_kick_patience_post_b10: 3,
    upload_max_retries: 5,
    client_alive_retry_ms: 30000,
    /**
     * Upload concurrency base (iss/70 P2: final cap = min of budgets, not max overlay).
     * Adaptive AIMD may raise up to upload_concurrency_max.
     */
    upload_concurrency: 3,
    upload_concurrency_max: 6,
    upload_mid_ramp: 4,
    upload_ramp_after: 4,
  };

  /**
   * Apply server open.policy.fe_retry (admin panel publish). Unknown keys ignored.
   */
  function applyServerPolicy(pol) {
    if (!pol || typeof pol !== "object") return POLICY;
    var keys = [
      "hard_max_attempts",
      "soft_max_attempts",
      "deepen_max_attempts",
      "rpa_max_attempts",
      "heavy_max_inflight",
      "fail_budget_n",
      "fail_budget_window_ms",
      "rpa_quiet_ms",
      "deepen_quiet_ms",
      "multi_tick_max",
      "empty_kick_patience",
      "empty_kick_patience_post_b10",
      "upload_max_retries",
      "client_alive_retry_ms",
      "upload_concurrency",
      "upload_concurrency_max",
      "upload_mid_ramp",
      "upload_ramp_after",
    ];
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i];
      if (typeof pol[k] === "number" && isFinite(pol[k]) && pol[k] > 0) {
        POLICY[k] = Math.floor(pol[k]);
      }
    }
    if (Array.isArray(pol.hard_sla_delays_ms) && pol.hard_sla_delays_ms.length) {
      POLICY.hard_sla_delays_ms = pol.hard_sla_delays_ms.map(function (x) {
        return Math.floor(Number(x) || 5000);
      });
    }
    return POLICY;
  }

  var failTimes = [];
  var state = STATE.NEED_PROBE;
  var quietUntil = 0;

  function now() {
    return Date.now();
  }

  function pruneFails() {
    var t = now();
    var w = POLICY.fail_budget_window_ms;
    failTimes = failTimes.filter(function (x) {
      return t - x < w;
    });
  }

  /**
   * Commercial hard anchors only — B10x silicon deepen is NOT hard.
   * (B10x used to match indexOf("B10") and burn hard_max / hard_exhausted.)
   */
  function isHardBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return false;
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    var hard = [
      "B10_hw_curves",
      "B0_bootstrap",
      "B2_hardware",
      "B3_system",
      "B1_conflict",
      "B12_anti_camouflage",
      "B8_gateway",
      "B8_gateway_early",
      "B7_sandbox",
    ];
    for (var i = 0; i < hard.length; i++) if (hard[i] === id) return true;
    return false;
  }

  /** Silicon deepen / multipath (scheduled by brain maximize; soft budget). */
  function isDeepenBatch(batchId) {
    var id = String(batchId || "");
    return id.indexOf("B10x_") === 0 || id === "B15_cross_curves";
  }

  /** Large body / GPU / SAB — serialize inflight (avoid tab freeze / WebGL thrash). */
  function isHeavyBatch(batchId) {
    var id = String(batchId || "");
    if (isDeepenBatch(id)) return true;
    return (
      id === "B7_sandbox" ||
      id === "B10_hw_curves" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B47_sab_clock" ||
      id === "B22_gpu_timer" ||
      id === "mid.curves"
    );
  }

  function isRpaBatch(batchId) {
    var id = String(batchId || "");
    return id === "B11_interaction" || id.indexOf("B11_") === 0 || id === "rpa.interaction";
  }

  var rpaQuietUntil = 0;
  var deepenQuietUntil = 0;

  function maxAttemptsFor(batchId) {
    if (isRpaBatch(batchId)) return POLICY.rpa_max_attempts || 2;
    if (isHardBatch(batchId)) return POLICY.hard_max_attempts;
    if (isDeepenBatch(batchId)) return POLICY.deepen_max_attempts || 4;
    return POLICY.soft_max_attempts;
  }

  function heavyMaxInflight() {
    return POLICY.heavy_max_inflight || 1;
  }

  function isCommercialLandFast(batchId) {
    var id = String(batchId || "");
    return (
      id === "B10_hw_curves" ||
      id === "mid.curves" ||
      id === "B10x_silicon_noderiv" ||
      id === "B10x_silicon_rint" ||
      id === "B10x_silicon_ulp"
    );
  }

  function backoffMsFor(batchId, attempt) {
    var a = Math.max(1, attempt | 0);
    var arr;
    if (isCommercialLandFast(batchId)) {
      arr = POLICY.commercial_land_backoff_ms || POLICY.hard_backoff_ms;
    } else if (isRpaBatch(batchId)) {
      arr = POLICY.rpa_backoff_ms || POLICY.soft_backoff_ms;
    } else if (isHardBatch(batchId)) {
      arr = POLICY.hard_backoff_ms;
    } else if (isDeepenBatch(batchId)) {
      arr = POLICY.deepen_backoff_ms || POLICY.soft_backoff_ms;
    } else {
      arr = POLICY.soft_backoff_ms;
    }
    var idx = Math.min(arr.length - 1, a - 1);
    var base = arr[idx] || 2000;
    // small jitter so multi-tab does not sync-storm (skip for first commercial land)
    var j =
      isCommercialLandFast(batchId) && a <= 2
        ? 0
        : Math.floor(base * 0.15 * Math.random());
    return base + j;
  }

  function recordFail(batchId, reason) {
    pruneFails();
    var why = String(reason || "");
    // Timeout aborts / pagehide / soft quiet: do NOT burn fail budget (was flooding
    // probe_fail_budget and blocking mid packs while hard still retrying).
    var softReason =
      /upload_timeout_abort|timeout|pagehide|halt|abort|network/i.test(why) &&
      !/attempts_exhausted/i.test(why);
    if (!softReason) {
      failTimes.push(now());
    }
    // RPA exhaust: quiet only B11 class — do not poison whole soft budget.
    if (why === "attempts_exhausted" && isRpaBatch(batchId)) {
      rpaQuietUntil = now() + (POLICY.rpa_quiet_ms || 90000);
      try {
        if (global.GROps && GROps.report) {
          GROps.report(
            "rpa_quiet",
            "upload",
            {
              batch_id: String(batchId || ""),
              quiet_ms: POLICY.rpa_quiet_ms || 90000,
              reason: why,
            },
            "warn"
          );
        }
      } catch (eR) {}
    }
    // Deepen exhaust / network storm: quiet B10x class so brain re-plan does not spin.
    if (
      (why === "attempts_exhausted" || /network|upload_network/i.test(why)) &&
      isDeepenBatch(batchId)
    ) {
      deepenQuietUntil = now() + (POLICY.deepen_quiet_ms || 120000);
    }
    var over = failTimes.length >= POLICY.fail_budget_n;
    if (over) {
      state = STATE.FAILED_QUIET;
      quietUntil = now() + POLICY.fail_budget_window_ms;
      try {
        // Dedupe ops: at most one probe_fail_budget per window.
        var lastPb = global.__GR_LAST_FAIL_BUDGET_MS__ || 0;
        var tNow = now();
        if (tNow - lastPb > POLICY.fail_budget_window_ms / 2 && global.GROps && GROps.report) {
          global.__GR_LAST_FAIL_BUDGET_MS__ = tNow;
          GROps.report(
            "probe_fail_budget",
            "upload",
            {
              n: failTimes.length,
              window_ms: POLICY.fail_budget_window_ms,
              last_batch: String(batchId || ""),
              reason: why,
            },
            "warn"
          );
        }
      } catch (e) {}
    } else if (!softReason) {
      state = STATE.RETRY_BACKOFF;
    }
    return { overBudget: over, failCount: failTimes.length, rpaQuiet: now() < rpaQuietUntil, soft: softReason };
  }

  function softQuietActive() {
    if (state === STATE.FAILED_QUIET && now() < quietUntil) return true;
    if (state === STATE.FAILED_QUIET && now() >= quietUntil) {
      state = STATE.NEED_PROBE;
      quietUntil = 0;
      failTimes = [];
    }
    return false;
  }

  function rpaQuietActive() {
    if (rpaQuietUntil && now() < rpaQuietUntil) return true;
    if (rpaQuietUntil && now() >= rpaQuietUntil) rpaQuietUntil = 0;
    return false;
  }

  function deepenQuietActive() {
    if (deepenQuietUntil && now() < deepenQuietUntil) return true;
    if (deepenQuietUntil && now() >= deepenQuietUntil) deepenQuietUntil = 0;
    return false;
  }

  /** iss/54–57 secondary silicon/infra — never quiet-block (ok or honest skip must land). */
  function isSecondaryInfraBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    );
  }

  /**
   * Soft packs should not enqueue while fail-budget quiet; hard still may.
   * RPA / deepen have class quiet. Heavy deepen re-queues only after quiet.
   */
  function isTimingSensitiveBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B34_cpu_cache_ladder" ||
      id === "B22_gpu_timer" ||
      id === "B37_thermal_drift_lite" ||
      id === "B42_thermal_drift_full" ||
      /timing|thermal|sab_clock|cache_ladder|eu_timing/i.test(id)
    );
  }

  function allowEnqueue(batchId) {
    if (isSecondaryInfraBatch(batchId)) return true;
    if (isRpaBatch(batchId) && rpaQuietActive()) return false;
    // iss/58 A9: defer timing-sensitive packs under serious/critical pressure
    if (isTimingSensitiveBatch(batchId) && !timingProbesAllowed()) return false;
    // silicon_deep is deepen but must not be blocked by ulp/noderiv quiet
    if (isDeepenBatch(batchId) && deepenQuietActive() && String(batchId || "") !== "B10x_silicon_deep") {
      return false;
    }
    if (softQuietActive() && !isHardBatch(batchId) && !isDeepenBatch(batchId)) return false;
    return true;
  }

  function markProbing() {
    if (state !== STATE.COMPLETE_COOL && state !== STATE.IDLE_COOL) {
      state = STATE.PROBING;
    }
  }

  /**
   * iss/58 A9: Compute Pressure Observer — no user prompt.
   * Gates timing-heavy packs when pressure is serious/critical.
   */
  function installComputePressure() {
    try {
      if (global.__GR_COMPUTE_PRESSURE_INSTALLED__) return;
      global.__GR_COMPUTE_PRESSURE_INSTALLED__ = 1;
      global.__GR_COMPUTE_PRESSURE_STATE__ = "unknown";
      var PO = global.PressureObserver || global.ComputePressureObserver;
      if (!PO) return;
      var obs = new PO(function (records) {
        try {
          var last = records && records.length ? records[records.length - 1] : null;
          var st = (last && (last.state || last.pressure)) || "unknown";
          global.__GR_COMPUTE_PRESSURE_STATE__ = String(st);
        } catch (eR) {}
      });
      // Observe CPU if supported
      try {
        if (obs.observe.length >= 1) {
          var p = obs.observe("cpu");
          if (p && p.catch) p.catch(function () {});
        } else {
          obs.observe({ source: "cpu" });
        }
      } catch (eO) {
        try {
          obs.observe();
        } catch (eO2) {}
      }
    } catch (e) {}
  }

  function timingProbesAllowed() {
    try {
      var st = String(global.__GR_COMPUTE_PRESSURE_STATE__ || "unknown").toLowerCase();
      if (st === "serious" || st === "critical") return false;
    } catch (e) {}
    return true;
  }

  try {
    installComputePressure();
  } catch (eInst) {}

  function markCool(source) {
    state = STATE.COMPLETE_COOL;
    try {
      global.__GR_PHASE__ = "cool";
      global.__GR_BUSINESS_STATE__ = "identity_complete_cool";
      global.__GR_LIFECYCLE_SOURCE__ = source || "cool";
    } catch (e) {}
  }

  function markNeedProbe(reason) {
    state = STATE.NEED_PROBE;
    quietUntil = 0;
    try {
      global.__GR_LIFECYCLE_REASON__ = reason || "need_probe";
    } catch (e) {}
  }

  function deriveFromOpen(opened) {
    opened = opened || {};
    var phase = String(opened.phase || "");
    var skip = !!(opened.skip_identity_probe || opened.skip_session_probe);
    var cps = opened.cycle_probe_status || {};
    var scheduleFinal =
      cps.brain_schedule_final === true ||
      cps.final_analysis_ok === true ||
      cps.cycle_status === "complete" ||
      opened.cycle_status === "complete" ||
      opened.brain_schedule_final === true;
    // Cool only when skip/phase cool AND schedule final (or explicit identity_complete_cool).
    // Thin phase=cool without schedule final → keep probing (gap-fill).
    if (
      (phase === "cool" || skip || opened.business_state === "identity_complete_cool") &&
      (scheduleFinal || opened.business_state === "identity_complete_cool") &&
      !opened.need_hard_anchor &&
      !opened.force_identity_probe
    ) {
      markCool("open");
      return STATE.IDLE_COOL;
    }
    markProbing();
    return STATE.PROBING;
  }

  function snapshot() {
    pruneFails();
    return {
      state: state,
      quiet_until_ms: quietUntil,
      fail_count_window: failTimes.length,
      policy: {
        hard_max_attempts: POLICY.hard_max_attempts,
        soft_max_attempts: POLICY.soft_max_attempts,
        fail_budget_n: POLICY.fail_budget_n,
        multi_tick_max: POLICY.multi_tick_max,
      },
    };
  }

  /**
   * Multi-path re-probe arming (maximize schedule).
   * Triggers: pageshow/BFCache, visibility, focus, storage (other tab),
   * online, cold→hot open force, explicit server route packs.
   * Does NOT stop because commercial device_id already minted.
   */
  var reprobeHooksInstalled = false;
  function installReprobeTriggers(kickFn) {
    if (reprobeHooksInstalled) return;
    reprobeHooksInstalled = true;
    var kick =
      typeof kickFn === "function"
        ? kickFn
        : function (reason) {
            try {
              if (typeof global.grResumeProbeCycle === "function") {
                global.grResumeProbeCycle(reason || "reprobe");
              } else if (typeof global.grKickRoutePlan === "function") {
                global.grKickRoutePlan(reason || "reprobe");
              } else if (global.GRBoot && typeof global.GRBoot.resumeProbe === "function") {
                global.GRBoot.resumeProbe(reason || "reprobe");
              } else {
                global.__GR_NEED_REPROBE__ = true;
                global.__GR_REPROBE_REASON__ = reason || "reprobe";
                markNeedProbe(reason || "reprobe");
              }
            } catch (e) {
              markNeedProbe(reason || "reprobe");
            }
          };
    var lastArmMs = 0;
    var pendingArmReason = null;
    function isBusy() {
      try {
        if (global.__GR_MULTI_TICK_ACTIVE__) return true;
        if (global.GRUploadQueue && GRUploadQueue.stats) {
          var st = GRUploadQueue.stats() || {};
          if ((st.pending || 0) > 0 || (st.inflight || 0) > 0) return true;
        }
      } catch (eB) {}
      return false;
    }
    function arm(reason) {
      try {
        // Never cool solely because a previous session had mint — server schedule owns cool.
        if (state === STATE.COMPLETE_COOL && !global.__GR_FORCE_REPROBE__) {
          return;
        }
        // iss/72 complete form: prefer GRSessionScheduler as single arm/kick arbiter.
        if (global.GRSessionScheduler && typeof GRSessionScheduler.submit === "function") {
          try {
            if (!GRSessionScheduler._lifecycleBound) {
              GRSessionScheduler.bind({
                kick: function (r) {
                  try {
                    markNeedProbe(r);
                    markProbing();
                  } catch (eM) {}
                  kick(r);
                },
                debounce_ms: 5000,
              });
              GRSessionScheduler._lifecycleBound = true;
            }
          } catch (eBind) {}
          var force =
            reason === "server_incomplete" ||
            reason === "route_plan" ||
            !!global.__GR_FORCE_REPROBE__;
          var ep = null;
          try {
            if (global.__GR_PLAN_EPOCH__ != null) ep = Number(global.__GR_PLAN_EPOCH__);
          } catch (eEp) {}
          var sub = GRSessionScheduler.submit(reason || "reprobe", {
            force: force,
            plan_epoch: ep,
          });
          if (sub && sub.ok === false && (sub.reason === "busy" || sub.reason === "debounce")) {
            pendingArmReason = reason || sub.reason;
          }
          return;
        }
        // Fallback lite path when session_scheduler not in pack.
        var now = Date.now();
        if (isBusy()) {
          pendingArmReason = reason || "busy_pending";
          return;
        }
        if (now - lastArmMs < 5000 && reason !== "server_incomplete") {
          pendingArmReason = reason || "debounce";
          return;
        }
        lastArmMs = now;
        pendingArmReason = null;
        markNeedProbe(reason);
        markProbing();
        kick(reason);
      } catch (e) {}
    }
    try {
      global.addEventListener("pageshow", function (ev) {
        arm(ev && ev.persisted ? "bfcache_pageshow" : "pageshow");
      });
      global.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "visible") arm("visibility_visible");
      });
      global.addEventListener("focus", function () {
        arm("window_focus");
      });
      global.addEventListener("online", function () {
        arm("network_online");
      });
      global.addEventListener("storage", function (ev) {
        if (ev && String(ev.key || "").indexOf("gr") >= 0) arm("storage_cross_tab");
      });
      // Periodic soft re-check while incomplete (server may re-arm packs on cold→hot).
      setInterval(function () {
        try {
          if (global.__GR_HALT_UPLOADS__ || global.__GR_BRAIN_SCHEDULE_FINAL__) return;
          if (state === STATE.COMPLETE_COOL) return;
          if (document.visibilityState !== "visible") return;
          // Flush pending arm reason if idle (scheduler tick also drains).
          if (pendingArmReason && !isBusy()) {
            var r = pendingArmReason;
            pendingArmReason = null;
            arm(r);
            return;
          }
          if (global.GRSessionScheduler && typeof GRSessionScheduler.tick === "function") {
            try {
              GRSessionScheduler.tick();
            } catch (eT) {}
          }
          arm("interval_maximize_probe");
        } catch (e) {}
      }, 45000);
    } catch (e) {}
    global.GRProbeLifecycle._armReprobe = arm;
  }

  /** Server open/result said incomplete → force need_probe (cold→hot rearm). */
  function onServerProbeStatus(cps) {
    cps = cps || {};
    var incomplete =
      cps.brain_schedule_final !== true &&
      cps.final_analysis_ok !== true &&
      cps.cycle_status !== "complete";
    var force =
      cps.force_identity_probe === true ||
      cps.need_hard_anchor === true ||
      cps.cold_to_hot === true ||
      cps.accepts_identity_ingest === true;
    if (incomplete && force) {
      markNeedProbe(cps.reprobe_reason || "server_incomplete");
      markProbing();
      try {
        global.__GR_FORCE_REPROBE__ = true;
        if (global.GRProbeLifecycle && global.GRProbeLifecycle._armReprobe) {
          global.GRProbeLifecycle._armReprobe(cps.reprobe_reason || "server_incomplete");
        }
      } catch (e) {}
      return true;
    }
    return false;
  }

  global.GRProbeLifecycle = {
    STATE: STATE,
    POLICY: POLICY,
    applyServerPolicy: applyServerPolicy,
    isHardBatch: isHardBatch,
    isDeepenBatch: isDeepenBatch,
    isHeavyBatch: isHeavyBatch,
    isCommercialLandFast: isCommercialLandFast,
    isRpaBatch: isRpaBatch,
    maxAttemptsFor: maxAttemptsFor,
    heavyMaxInflight: heavyMaxInflight,
    backoffMsFor: backoffMsFor,
    recordFail: recordFail,
    softQuietActive: softQuietActive,
    rpaQuietActive: rpaQuietActive,
    deepenQuietActive: deepenQuietActive,
    allowEnqueue: allowEnqueue,
    markProbing: markProbing,
    markCool: markCool,
    markNeedProbe: markNeedProbe,
    deriveFromOpen: deriveFromOpen,
    snapshot: snapshot,
    installReprobeTriggers: installReprobeTriggers,
    installComputePressure: installComputePressure,
    timingProbesAllowed: timingProbesAllowed,
    onServerProbeStatus: onServerProbeStatus,
    getState: function () {
      return state;
    },
  };

  // Auto-install multi-path re-probe hooks as soon as lifecycle loads.
  try {
    installReprobeTriggers(null);
  } catch (e0) {}
  try {
    installComputePressure();
  } catch (e1) {}
})(typeof window !== "undefined" ? window : globalThis);

/* ---- session_scheduler.js ---- */
/**
 * GR SessionScheduler — single arm/kick arbiter (iss/72 complete form).
 *
 * All lifecycle / hard-SLA / route-plan / multi-tick re-arms submit events here.
 * Scheduler decides: debounce, busy, cool/terminal, plan_epoch freshness, then kick.
 */
(function (global) {
  "use strict";

  var sessionId = "";
  var planEpoch = 0;
  var lastArmMs = 0;
  var pending = null; // { reason, epoch, force }
  var debounceMs = 5000;
  var handlers = {
    kick: null,
    onEvent: null,
  };
  var stats = {
    events: 0,
    kicked: 0,
    deferred_busy: 0,
    deferred_debounce: 0,
    dropped_stale_epoch: 0,
    dropped_cool: 0,
  };

  function now() {
    return Date.now();
  }

  function isCool() {
    try {
      if (global.__GR_BRAIN_SCHEDULE_FINAL__ || global.__GR_HARD_FINAL__) return true;
      if (global.__GR_HALT_UPLOADS__ && global.__GR_SKIP_IDENTITY__) return true;
      if (global.GRProbeLifecycle && GRProbeLifecycle.getState) {
        var st = GRProbeLifecycle.getState();
        if (st === "complete_cool" || st === "idle_cool") return true;
      }
    } catch (e) {}
    return false;
  }

  function uploadIsBusy() {
    try {
      if (global.GRUploadQueue && GRUploadQueue.stats) {
        var s = GRUploadQueue.stats() || {};
        if ((s.pending || 0) > 0 || (s.inflight || 0) > 0) return true;
      }
    } catch (e) {}
    return false;
  }

  // Uploading is intentionally not a global collection lock. Captures are
  // frozen before enqueue, so an acknowledged/sealed transport can overlap
  // the next value-ranked collection kick. The collection arbiter remains
  // busy only while a probe cycle is actively executing.
  function isBusy() {
    try {
      return !!global.__GR_MULTI_TICK_ACTIVE__;
    } catch (e) {
      return false;
    }
  }

  function setSession(sid) {
    if (sid && String(sid) !== String(sessionId || "")) {
      sessionId = String(sid);
      planEpoch = 0;
      pending = null;
    } else if (sid) {
      sessionId = String(sid);
    }
  }

  function setPlanEpoch(ep) {
    var n = Number(ep);
    if (!isFinite(n) || n < 0) return planEpoch;
    if (n > planEpoch) planEpoch = n;
    return planEpoch;
  }

  function getPlanEpoch() {
    return planEpoch;
  }

  /**
   * Submit a scheduler event. Does not kick immediately when busy/debounced.
   * @param {string} reason
   * @param {object} [opts] { force, plan_epoch, kick }
   */
  function submit(reason, opts) {
    opts = opts || {};
    stats.events++;
    var force = !!opts.force;
    var ep = opts.plan_epoch != null ? Number(opts.plan_epoch) : null;

    // Stale plan: client tries to act on older epoch than known.
    if (ep != null && isFinite(ep) && ep > 0 && ep < planEpoch && !force) {
      stats.dropped_stale_epoch++;
      try {
        if (handlers.onEvent) handlers.onEvent("stale_epoch", { reason: reason, ep: ep, planEpoch: planEpoch });
      } catch (e) {}
      return { ok: false, reason: "stale_plan_epoch", plan_epoch: planEpoch };
    }
    if (ep != null && isFinite(ep) && ep > planEpoch) {
      planEpoch = ep;
    }

    if (isCool() && !force && !global.__GR_FORCE_REPROBE__) {
      stats.dropped_cool++;
      return { ok: false, reason: "cool" };
    }

    var t = now();
    if (!force && (isBusy() || (opts.wait_for_upload && uploadIsBusy()))) {
      pending = { reason: reason || "busy", epoch: planEpoch, force: false, at: t };
      stats.deferred_busy++;
      return { ok: false, reason: "busy", pending: true };
    }
    if (!force && t - lastArmMs < debounceMs && reason !== "server_incomplete" && reason !== "route_plan") {
      pending = { reason: reason || "debounce", epoch: planEpoch, force: false, at: t };
      stats.deferred_debounce++;
      return { ok: false, reason: "debounce", pending: true };
    }

    return execute(reason || "submit", force);
  }

  function execute(reason, force) {
    lastArmMs = now();
    pending = null;
    stats.kicked++;
    try {
      if (global.GRProbeLifecycle) {
        if (GRProbeLifecycle.markNeedProbe) GRProbeLifecycle.markNeedProbe(reason);
        if (GRProbeLifecycle.markProbing) GRProbeLifecycle.markProbing();
      }
    } catch (eM) {}
    try {
      if (typeof handlers.kick === "function") {
        handlers.kick(reason, { force: !!force, plan_epoch: planEpoch, session_id: sessionId });
      } else if (typeof global.grResumeProbeCycle === "function") {
        global.grResumeProbeCycle(reason);
      } else if (typeof global.grKickRoutePlan === "function") {
        global.grKickRoutePlan(reason);
      } else if (global.GRBoot && typeof global.GRBoot.resumeProbe === "function") {
        global.GRBoot.resumeProbe(reason);
      }
    } catch (eK) {}
    return { ok: true, reason: reason, plan_epoch: planEpoch };
  }

  /** Flush deferred event if idle. */
  function tick() {
    if (!pending) return null;
    if (isCool() && !pending.force) {
      pending = null;
      return null;
    }
    if (isBusy()) return null;
    var p = pending;
    pending = null;
    return execute(p.reason, p.force);
  }

  /**
   * Apply route_plan only if epoch is fresh.
   * @returns {{ok:boolean, reason?:string, plan_epoch?:number}}
   */
  function acceptRoutePlan(plan, analysisRev) {
    plan = plan || {};
    var ep =
      plan.plan_epoch != null
        ? Number(plan.plan_epoch)
        : plan.plan_version != null
          ? Number(plan.plan_version)
          : analysisRev != null
            ? Number(analysisRev)
            : 0;
    if (!isFinite(ep) || ep <= 0) {
      // No epoch — accept but don't regress
      return { ok: true, plan_epoch: planEpoch, reason: "no_epoch" };
    }
    if (ep < planEpoch) {
      stats.dropped_stale_epoch++;
      return { ok: false, reason: "stale_plan_epoch", plan_epoch: planEpoch, got: ep };
    }
    planEpoch = ep;
    try {
      global.__GR_PLAN_EPOCH__ = planEpoch;
    } catch (e) {}
    return { ok: true, plan_epoch: planEpoch };
  }

  function bind(opts) {
    opts = opts || {};
    if (typeof opts.kick === "function") handlers.kick = opts.kick;
    if (typeof opts.onEvent === "function") handlers.onEvent = opts.onEvent;
    if (opts.debounce_ms != null) debounceMs = Math.max(0, Number(opts.debounce_ms) || 0);
    if (opts.session_id) setSession(opts.session_id);
  }

  function snapshot() {
    return {
      session_id: sessionId,
      plan_epoch: planEpoch,
      pending: pending,
      debounce_ms: debounceMs,
      busy: isBusy(),
      upload_busy: uploadIsBusy(),
      cool: isCool(),
      stats: {
        events: stats.events,
        kicked: stats.kicked,
        deferred_busy: stats.deferred_busy,
        deferred_debounce: stats.deferred_debounce,
        dropped_stale_epoch: stats.dropped_stale_epoch,
        dropped_cool: stats.dropped_cool,
      },
    };
  }

  // Drain pending every few seconds
  try {
    setInterval(function () {
      try {
        tick();
      } catch (e) {}
    }, 2000);
  } catch (eI) {}

  global.GRSessionScheduler = {
    bind: bind,
    setSession: setSession,
    setPlanEpoch: setPlanEpoch,
    getPlanEpoch: getPlanEpoch,
    submit: submit,
    tick: tick,
    acceptRoutePlan: acceptRoutePlan,
    isBusy: isBusy,
    uploadIsBusy: uploadIsBusy,
    isCool: isCool,
    snapshot: snapshot,
  };
})(typeof window !== "undefined" ? window : globalThis);

/* ---- origin_coordinator.js ---- */
/**
 * GR Origin Coordinator — same-origin multi-tab heavy-role intelligence.
 *
 * Scope: ONE browser origin (shop.example.com tabs share this; news.example.com is separate).
 *
 * Roles:
 *   heavy     — may run B10 / hard silicon collect
 *   light     — L1 + RPA + transport only (no heavy re-collect)
 *   transport — flush queue / gap-fill uploads only
 *
 * Mechanisms:
 *   1) navigator.locks (preferred) for atomic heavy lease
 *   2) localStorage lease + heartbeat (fallback + cross-tab visibility)
 *   3) BroadcastChannel for progress / handoff / cool (immediate, best-effort)
 *   4) Durable handoff ticket in localStorage (survives closing tab; storage event
 *      + poll claim) — industry pattern: BC alone drops if no live listener;
 *      GA4/Sentry use beacon for payloads; task *continuation* needs shared durable state
 *      (RxDB leader + IDB; we use LS ticket + Web Locks for lighter footprint).
 *
 * Handoff:
 *   - pagehide/beforeunload: release heavy + write durable unfinished ticket + BC
 *   - survivors (or same-tab multipage nav resume): claim ticket → upgrade heavy → ensureGap
 *   - clear ticket when server has B10 / work satisfied
 *
 * @see reports/FE_PROBE_GLOBAL_INTELLIGENCE_ARCH_V1.md
 * @see reports/MULTI_TAB_HANDOFF_RESEARCH_V1.md
 */
(function (global) {
  "use strict";

  if (global.GROriginCoordinator && global.GROriginCoordinator.__ready) return;

  var CH_NAME = "gr-gpi-v1";
  var LS_LEASE = "_g5_gpi_lease";
  var LS_PROGRESS = "_g5_gpi_prog";
  var LS_HANDOFF = "_g5_gpi_handoff";
  var LS_PEERS = "_g5_gpi_peers";
  var LOCK_NAME = "gr-heavy-b10";
  var HEARTBEAT_MS = 2000;
  var LEASE_TTL_MS = 6000;
  var UPGRADE_AFTER_MS = 10000;
  var PROGRESS_STALE_MS = 12000;
  var HANDOFF_TTL_MS = 120000; // 2 min — claim window after tab close / route change
  var HANDOFF_CLAIM_STALE_MS = 15000; // re-claim if claimer died mid-upgrade
  var PEER_TTL_MS = 10000;

  var tabId = null;
  var role = "light";
  var started = false;
  var bc = null;
  var hbTimer = null;
  var watchTimer = null;
  var lockHeld = false;
  var lockRelease = null;
  var cycleId = "";
  var claimingHandoff = false;
  var stats = {
    becomes_heavy: 0,
    becomes_light: 0,
    handoffs_sent: 0,
    handoffs_recv: 0,
    handoffs_durable_write: 0,
    handoffs_claim: 0,
    upload_handoffs: 0,
    upgrades: 0,
    heartbeats: 0,
    peer_max: 1,
  };

  function now() {
    return Date.now();
  }

  function mintTabId() {
    try {
      var k = "__gr_tab_win_id__";
      var ss = global.sessionStorage;
      if (ss) {
        var ex = ss.getItem(k);
        if (ex) return ex;
        var id = "t" + Math.random().toString(36).slice(2, 10) + now().toString(36).slice(-4);
        ss.setItem(k, id);
        return id;
      }
    } catch (e) {}
    return "t" + Math.random().toString(36).slice(2, 12);
  }

  function debugOn() {
    try {
      return !!(
        global.__GR_DEBUG_GPI__ ||
        global.__GR_DEBUG__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.debug_gpi)
      );
    } catch (e) {
      return false;
    }
  }

  function clog(level, msg, detail) {
    if (!debugOn()) return;
    try {
      var fn = (global.console && (global.console[level] || global.console.log)) || null;
      if (!fn) return;
      var line = "[gr:gpi] " + msg;
      if (detail != null) fn.call(global.console, line, detail);
      else fn.call(global.console, line);
    } catch (e) {}
  }

  function ops(code, detail, sev) {
    try {
      if (global.GROps && GROps.report) {
        GROps.report(code, "origin_coord", detail || {}, sev || "info");
      }
    } catch (e) {}
    clog(sev === "warn" || sev === "error" ? "warn" : "info", code, detail || null);
  }

  function timeline(ev, d) {
    try {
      var tl = global.__GR_PROBE_TIMELINE__;
      if (!Array.isArray(tl)) {
        tl = [];
        global.__GR_PROBE_TIMELINE__ = tl;
      }
      tl.push({ t: now(), ev: "gpi_" + ev, d: d || null });
      if (tl.length > 100) tl.splice(0, tl.length - 100);
    } catch (e) {}
    clog("debug", "timeline " + ev, d || null);
  }

  function readJson(key) {
    try {
      var s = global.localStorage && localStorage.getItem(key);
      if (!s) return null;
      return JSON.parse(s);
    } catch (e) {
      return null;
    }
  }

  function writeJson(key, obj) {
    try {
      if (global.localStorage) localStorage.setItem(key, JSON.stringify(obj));
    } catch (e) {}
  }

  function clearKey(key) {
    try {
      if (global.localStorage) localStorage.removeItem(key);
    } catch (e) {}
  }

  function sid() {
    try {
      return String(
        cycleId ||
          global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.GRUploadQueue && GRUploadQueue.cfg && GRUploadQueue.cfg.session_id) ||
          ""
      );
    } catch (e) {
      return cycleId || "";
    }
  }

  function prunePeers(map) {
    var out = map && typeof map === "object" ? map : {};
    var t = now();
    Object.keys(out).forEach(function (k) {
      var p = out[k];
      if (!p || t - Number(p.hb || 0) > PEER_TTL_MS) delete out[k];
    });
    return out;
  }

  function touchPeer() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    all[tabId] = {
      hb: now(),
      role: role,
      cycle: sid(),
      href: (function () {
        try {
          return String(location.pathname || "").slice(0, 80);
        } catch (e) {
          return "";
        }
      })(),
    };
    writeJson(LS_PEERS, all);
    var n = Object.keys(all).length;
    if (n > stats.peer_max) stats.peer_max = n;
    return n;
  }

  function dropPeer() {
    try {
      var all = prunePeers(readJson(LS_PEERS) || {});
      delete all[tabId];
      writeJson(LS_PEERS, all);
    } catch (e) {}
  }

  function peerCount() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    return Object.keys(all).length || (started ? 1 : 0);
  }

  /**
   * Multi-tab brain policy — consumed by self-heal / pack_loader / upload.
   * More tabs ⇒ focus heavy on silicon; light tabs = transport + RPA only.
   */
  function policySnapshot() {
    var n = peerCount();
    var multi = n > 1;
    return {
      peer_count: n,
      multi_tab: multi,
      role: role,
      heavy: role === "heavy",
      // light tabs: transport-first (PostHog/Sentry-style queue drain)
      transport_assist: multi && role !== "heavy",
      // heavy under multi-tab: serialize silicon, less deepen thrash
      serial_heavy: multi && role === "heavy",
      // allow light to skip hard collect (pack_loader already gates)
      defer_heavy_collect: multi && role !== "heavy",
      // slightly faster transport nudge when peers help
      transport_priority: multi ? (role === "heavy" ? 1 : 2) : 1,
    };
  }

  function publishRole() {
    try {
      var pol = policySnapshot();
      global.__GR_GPI_ROLE__ = role;
      global.__GR_GPI_TAB_ID__ = tabId;
      global.__GR_GPI_HEAVY__ = role === "heavy";
      global.__GR_GPI_PEER_COUNT__ = pol.peer_count;
      global.__GR_GPI_POLICY__ = pol;
      global.__GR_GPI_MULTI_TAB__ = !!pol.multi_tab;
    } catch (e) {}
  }

  function setRole(next, why) {
    next = String(next || "light");
    if (next !== "heavy" && next !== "light" && next !== "transport") next = "light";
    if (role === next) {
      publishRole();
      return role;
    }
    var prev = role;
    role = next;
    publishRole();
    if (next === "heavy") stats.becomes_heavy++;
    else stats.becomes_light++;
    timeline("role", { from: prev, to: next, why: why || "" });
    ops(
      "gpi_role",
      { tab: tabId, from: prev, to: next, why: why || "", cycle: sid().slice(0, 28) },
      "info"
    );
    broadcast({ type: "role", tab: tabId, role: role, cycle: sid(), why: why || "" });
    // Nudge self-heal to re-evaluate collect rights immediately.
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
    } catch (eT) {}
    return role;
  }

  function broadcast(msg) {
    try {
      if (bc) bc.postMessage(Object.assign({ t: now(), from: tabId }, msg || {}));
    } catch (e) {}
  }

  function leaseSnapshot() {
    return readJson(LS_LEASE);
  }

  function leaseIsMine(L) {
    return !!(L && L.tab === tabId);
  }

  function leaseAlive(L) {
    if (!L || !L.tab) return false;
    var hb = Number(L.hb || L.at || 0);
    return now() - hb < LEASE_TTL_MS;
  }

  function writeLease(extra) {
    var L = Object.assign(
      {
        tab: tabId,
        at: now(),
        hb: now(),
        cycle: sid(),
      },
      extra || {}
    );
    writeJson(LS_LEASE, L);
    return L;
  }

  function clearMyLease() {
    var L = leaseSnapshot();
    if (leaseIsMine(L)) clearKey(LS_LEASE);
  }

  function writeProgress(patch) {
    var p = Object.assign(
      {
        tab: tabId,
        role: role,
        cycle: sid(),
        at: now(),
        has_b10_local: false,
        phase: "",
      },
      patch || {}
    );
    try {
      if (global.__GR_RECEIVED_BATCHES__) {
        /* noop */
      }
      var st =
        global.GRUploadQueue &&
        GRUploadQueue.materialState &&
        GRUploadQueue.materialState("B10_hw_curves", sid(), "main");
      if (st === "acked") p.has_b10_local = true;
    } catch (e) {}
    try {
      if (global.__GR_CYCLE_PROBE_STATUS__ && global.__GR_CYCLE_PROBE_STATUS__.has_b10) {
        p.has_b10_server = true;
      }
    } catch (e2) {}
    writeJson(LS_PROGRESS, p);
    broadcast({ type: "progress", progress: p });
    return p;
  }

  function serverHasB10() {
    try {
      var cps = global.__GR_CYCLE_PROBE_STATUS__ || {};
      if (cps.has_b10 === true) return true;
      var cov = cps.identity_coverage || {};
      if (cov.has_b10 === true) return true;
    } catch (e) {}
    try {
      var so = global.__GR_SESSION_OUTCOME__ || {};
      if (so.batches_ok && (so.batches_ok.B10_hw_curves || so.batches_ok["mid.curves"])) return true;
    } catch (e2) {}
    return false;
  }

  function canHeavyCollect() {
    if (serverHasB10()) return false;
    if (role === "heavy") return true;
    // No live lease and we need upgrade path
    var L = leaseSnapshot();
    if (!leaseAlive(L)) return role === "heavy";
    return false;
  }

  function isHeavy() {
    return role === "heavy";
  }

  function releaseHeavy(why) {
    if (lockRelease) {
      try {
        lockRelease();
      } catch (e) {}
      lockRelease = null;
    }
    lockHeld = false;
    clearMyLease();
    if (role === "heavy") setRole("light", why || "release");
    broadcast({ type: "lease_free", why: why || "release", tab: tabId });
  }

  function becomeHeavy(why) {
    writeLease({ why: why || "acquire" });
    lockHeld = true;
    setRole("heavy", why || "acquire");
    writeProgress({ role: "heavy", phase: "heavy_start" });
    stats.becomes_heavy++;
  }

  /**
   * Try to acquire heavy role. Async; resolves with boolean.
   */
  function tryAcquireHeavy(why) {
    return new Promise(function (resolve) {
      if (serverHasB10()) {
        setRole("light", "server_has_b10");
        resolve(false);
        return;
      }
      var L0 = leaseSnapshot();
      if (leaseAlive(L0) && !leaseIsMine(L0)) {
        setRole("light", "other_leader");
        resolve(false);
        return;
      }
      // Prefer Web Locks — if busy, stay light (do NOT fall through to storage race).
      try {
        if (global.navigator && navigator.locks && typeof navigator.locks.request === "function") {
          var settled = false;
          function finish(ok, whyF) {
            if (settled) return;
            settled = true;
            if (!ok) setRole("light", whyF || "lock_busy");
            resolve(!!ok);
          }
          navigator.locks
            .request(LOCK_NAME, { ifAvailable: true }, function (lock) {
              if (!lock) {
                finish(false, "lock_busy");
                return;
              }
              // Re-check storage lease after lock grant (another tab may have storage-only heavy).
              var L1 = leaseSnapshot();
              if (leaseAlive(L1) && !leaseIsMine(L1)) {
                finish(false, "storage_peer");
                return;
              }
              lockHeld = true;
              var released = false;
              lockRelease = function () {
                if (released) return;
                released = true;
              };
              becomeHeavy(why || "web_lock");
              finish(true, "web_lock");
              return new Promise(function (holdDone) {
                var iv = setInterval(function () {
                  if (!lockHeld || role !== "heavy") {
                    clearInterval(iv);
                    holdDone();
                  }
                }, 500);
              });
            })
            .catch(function () {
              // Locks API error → storage fallback only
              if (!settled) storageAcquire(why, resolve);
            });
          return;
        }
      } catch (eL) {}
      storageAcquire(why, resolve);
    });
  }

  function storageAcquire(why, resolve) {
    // Optimistic atomic-ish: re-read, write only if free/stale/mine
    var L = leaseSnapshot();
    if (leaseAlive(L) && !leaseIsMine(L)) {
      setRole("light", "storage_busy");
      resolve(false);
      return;
    }
    // Claim with random token to detect clobber
    var token = tabId + ":" + Math.random().toString(36).slice(2, 8);
    writeLease({ why: why || "storage_lease", token: token });
    // Verify we still own after write (last-writer check)
    var L2 = leaseSnapshot();
    if (!L2 || L2.tab !== tabId) {
      setRole("light", "storage_lost_race");
      resolve(false);
      return;
    }
    becomeHeavy(why || "storage_lease");
    resolve(true);
  }

  function heartbeat() {
    stats.heartbeats++;
    touchPeer();
    publishRole();
    // Nudge self-heal adaptive when multi-tab policy changes
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.applyAdaptivePolicy) {
        GRProbeSelfHeal.applyAdaptivePolicy(true);
      }
    } catch (eP) {}
    if (role === "heavy") {
      writeLease({ hb: now(), cycle: sid() });
      writeProgress({ role: "heavy", phase: "heartbeat", peers: peerCount() });
      if (serverHasB10()) {
        var Hu = readHandoff();
        if (!Hu || !Hu.upload_pending || !Hu.upload_pending.length) clearHandoff("heavy_server_b10");
      }
    } else {
      writeProgress({ role: role, phase: "follower", peers: peerCount() });
      maybeUpgrade();
      processHandoff("heartbeat");
      // Light multi-tab: actively assist transport (queue pump)
      try {
        var pol = policySnapshot();
        if (pol.transport_assist && global.GRUploadQueue && GRUploadQueue.flush) {
          // soft background flush — not unloading
          GRUploadQueue.flush("gpi_transport_assist");
        }
      } catch (eT) {}
    }
  }

  function maybeUpgrade() {
    if (role === "heavy") return;
    if (serverHasB10()) return;
    var L = leaseSnapshot();
    var prog = readJson(LS_PROGRESS);
    // Leader dead or progress stale → upgrade
    var leaderDead = !leaseAlive(L);
    var progressStale =
      !prog || !prog.at || now() - Number(prog.at) > PROGRESS_STALE_MS || prog.tab !== (L && L.tab);
    var waited = started ? now() - (global.__GR_GPI_START_MS__ || now()) : 0;
    if (leaderDead || (progressStale && waited > UPGRADE_AFTER_MS)) {
      tryAcquireHeavy(leaderDead ? "upgrade_lease_dead" : "upgrade_stale_progress").then(function (ok) {
        if (ok) {
          stats.upgrades++;
          timeline("upgrade", { why: leaderDead ? "lease_dead" : "stale" });
          ops("gpi_upgrade", { tab: tabId, why: leaderDead ? "lease_dead" : "stale" }, "warn");
        }
      });
    }
  }

  function onMessage(ev) {
    var m = (ev && ev.data) || {};
    if (!m || m.from === tabId) return;
    if (m.type === "progress" && m.progress) {
      try {
        // Cache peer progress for diagnostics
        global.__GR_GPI_PEER__ = m.progress;
      } catch (e) {}
      if (m.progress.has_b10_local || m.progress.has_b10_server) {
        if (role === "heavy" && !serverHasB10()) {
          /* keep heavy until server confirms — avoid flip-flop */
        } else if (role === "heavy" && serverHasB10()) {
          releaseHeavy("peer_or_server_b10");
        }
      }
    }
    if (m.type === "handoff") {
      stats.handoffs_recv++;
      timeline("handoff_recv", m);
      ops(
        "gpi_handoff_recv",
        {
          from: m.from,
          unfinished: m.unfinished || [],
          cycle: m.cycle,
          durable: !!m.durable,
        },
        "warn"
      );
      // Mirror into durable store if peer only BC'd (older path / race)
      if (Array.isArray(m.unfinished) && m.unfinished.length && !serverHasB10()) {
        var cur = readHandoff();
        if (!handoffIsActionable(cur)) {
          writeJson(LS_HANDOFF, {
            v: 1,
            from: m.from,
            cycle: m.cycle || sid(),
            unfinished: m.unfinished.slice(),
            at: now(),
            why: "bc_mirror",
            claimed_by: null,
            claimed_at: 0,
          });
        }
      }
      processHandoff("bc");
    }
    if (m.type === "cool" || m.type === "has_b10") {
      if (serverHasB10()) clearHandoff("peer_has_b10");
      if (role === "heavy") {
        // Demote after peer cool if server has b10
        if (serverHasB10()) releaseHeavy("peer_cool");
      }
    }
    if (m.type === "lease_free") {
      maybeUpgrade();
      processHandoff("lease_free");
    }
  }

  function collectUnfinished() {
    var out = [];
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.snapshot) {
        var snap = GRProbeSelfHeal.snapshot();
        (snap.gaps || []).forEach(function (g) {
          if (g && g.desired && !g.satisfied && (g.batch_id === "B10_hw_curves" || g.batch_id === "mid.curves")) {
            out.push(g.batch_id);
          }
        });
      }
    } catch (e) {}
    // Also inspect pack health — heavy mid-flight may not have gap entry yet
    try {
      var packs = global.__GR_PACK_HEALTH__ || {};
      var b10h = packs.B10_hw_curves || packs["mid.curves"];
      if (b10h && (b10h.run === "running" || b10h.run === "queued" || b10h.state === "running")) {
        if (out.indexOf("B10_hw_curves") < 0) out.push("B10_hw_curves");
      }
    } catch (e2) {}
    if (!out.length && !serverHasB10() && role === "heavy") out.push("B10_hw_curves");
    // de-dup
    var seen = {};
    return out.filter(function (x) {
      if (seen[x]) return false;
      seen[x] = 1;
      return true;
    });
  }

  /**
   * Pending upload batch ids (not full payloads — sealed bodies too large for LS).
   * Survivor re-collects if server missing; otherwise transport nudge only.
   * Mirrors PostHog retry-queue persistence of *request identity*, not always body.
   */
  function collectUploadPending() {
    var out = [];
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.stats === "function") {
        var st = q.stats() || {};
        var pend = st.pending_batches || st.pending_ids || st.batches_pending || null;
        if (Array.isArray(pend)) {
          pend.forEach(function (b) {
            if (b && out.indexOf(String(b)) < 0) out.push(String(b));
          });
        }
      }
      // material state scan for hard anchors still not acked
      if (q && typeof q.materialState === "function") {
        ["B10_hw_curves", "mid.curves", "B0_bootstrap", "B2_hardware", "B3_system"].forEach(function (bid) {
          try {
            var ms = q.materialState(bid, sid(), "main");
            if (ms && ms !== "acked" && ms !== "absent" && out.indexOf(bid) < 0) {
              if (ms === "queued" || ms === "uploading" || ms === "retry_wait" || ms === "collected" || ms === "new") {
                out.push(bid);
              }
            }
          } catch (eM) {}
        });
      }
    } catch (e) {}
    return out.slice(0, 24);
  }

  function readHandoff() {
    return readJson(LS_HANDOFF);
  }

  function handoffIsActionable(H) {
    if (!H) return false;
    var at = Number(H.at || 0);
    if (!at || now() - at > HANDOFF_TTL_MS) return false;
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    var needUpload = Array.isArray(H.upload_pending) && H.upload_pending.length > 0;
    if (!needCollect && !needUpload) return false;
    // Peer already claimed and still looks alive → leave it (collect path)
    if (needCollect && H.claimed_by && H.claimed_by !== tabId) {
      var cAt = Number(H.claimed_at || 0);
      var claimFresh = cAt && now() - cAt < HANDOFF_CLAIM_STALE_MS;
      var L = leaseSnapshot();
      var claimerHoldsLease = leaseAlive(L) && L.tab === H.claimed_by;
      if (claimerHoldsLease) return false;
      if (claimFresh && leaseAlive(L) && L.tab !== tabId) return false;
    }
    return true;
  }

  /**
   * Durable ticket: localStorage survives the dying tab.
   * BC is best-effort notify for already-open survivors.
   * Same-tab multipage navigation also pagehides — ticket lets the next document resume.
   */
  function writeDurableHandoff(unfinished, why, uploadPending) {
    uploadPending = uploadPending || [];
    var needCollect = unfinished && unfinished.length && !serverHasB10();
    var needUpload = uploadPending && uploadPending.length;
    if (!needCollect && !needUpload) return null;
    var ticket = {
      v: 2,
      from: tabId,
      cycle: sid(),
      unfinished: needCollect ? unfinished.slice() : [],
      upload_pending: needUpload ? uploadPending.slice(0, 24) : [],
      at: now(),
      why: why || "pagehide",
      href: (function () {
        try {
          return String(location.href || "").slice(0, 160);
        } catch (e) {
          return "";
        }
      })(),
      role_was: role,
      claimed_by: null,
      claimed_at: 0,
    };
    // Prefer keep fresher ticket if same cycle still open
    var prev = readHandoff();
    if (prev && (handoffIsActionable(prev) || (prev.upload_pending && prev.upload_pending.length)) && prev.cycle === ticket.cycle) {
      var merged = (prev.unfinished || []).slice();
      (ticket.unfinished || []).forEach(function (u) {
        if (merged.indexOf(u) < 0) merged.push(u);
      });
      ticket.unfinished = merged;
      var um = (prev.upload_pending || []).slice();
      (ticket.upload_pending || []).forEach(function (u) {
        if (um.indexOf(u) < 0) um.push(u);
      });
      ticket.upload_pending = um.slice(0, 24);
      if (prev.claimed_by && prev.claimed_by !== tabId) {
        ticket.claimed_by = prev.claimed_by;
        ticket.claimed_at = prev.claimed_at;
      }
    }
    writeJson(LS_HANDOFF, ticket);
    stats.handoffs_durable_write++;
    if (ticket.upload_pending && ticket.upload_pending.length) stats.upload_handoffs++;
    timeline("handoff_durable", {
      unfinished: ticket.unfinished,
      upload_pending: ticket.upload_pending,
      why: why || "pagehide",
    });
    ops(
      "gpi_handoff_durable",
      {
        unfinished: ticket.unfinished,
        upload_pending: ticket.upload_pending,
        tab: tabId,
        why: why || "pagehide",
      },
      "warn"
    );
    return ticket;
  }

  function clearHandoff(why) {
    var H = readHandoff();
    if (!H) return;
    // Only clear if we own claim or ticket is ours / satisfied
    if (H.claimed_by && H.claimed_by !== tabId && handoffIsActionable(H)) return;
    clearKey(LS_HANDOFF);
    timeline("handoff_clear", { why: why || "done" });
    clog("info", "handoff_clear", { why: why || "done" });
  }

  function kickUploadAssist(uploadPending, reason) {
    if (!uploadPending || !uploadPending.length) return;
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.nudgeTransport === "function") {
        uploadPending.forEach(function (bid) {
          try {
            q.nudgeTransport(String(bid), sid());
          } catch (eN) {}
        });
      }
      if (q && typeof q.flush === "function") {
        q.flush("handoff_assist");
      }
    } catch (eF) {}
    try {
      if (global.GRProbeSelfHeal) {
        uploadPending.forEach(function (bid) {
          if (!bid) return;
          // Server missing → recollect; else transport loop will settle.
          if (GRProbeSelfHeal.ensureGap) {
            GRProbeSelfHeal.ensureGap(String(bid), "main", {
              desired: true,
              force_recollect: !serverHasB10() && (bid === "B10_hw_curves" || bid === "mid.curves"),
              reason: reason || "upload_handoff",
            });
          }
        });
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
  }

  function kickHeavyWork(reason) {
    // Clear stale light-defer markers so pack_loader will re-enter B10 after upgrade.
    try {
      var ph = global.__GR_PACK_HEALTH__;
      if (ph && typeof ph === "object") {
        ["B10_hw_curves", "mid.curves"].forEach(function (k) {
          if (ph[k] && ph[k].run === "deferred_gpi_light") {
            ph[k].run = "handoff_pending";
            ph[k].handoff_reason = reason || "handoff";
          }
        });
      }
    } catch (ePh) {}
    try {
      if (global.GRProbeSelfHeal) {
        if (GRProbeSelfHeal.ensureGap) {
          GRProbeSelfHeal.ensureGap("B10_hw_curves", "main", {
            desired: true,
            force_recollect: true,
            reason: reason || "handoff",
          });
        }
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
    try {
      var H = readHandoff();
      if (H && H.upload_pending) kickUploadAssist(H.upload_pending, reason);
    } catch (eU) {}
  }

  /**
   * Claim durable handoff and upgrade to heavy if needed.
   * Idempotent; safe to call from heartbeat / storage / BC / start.
   */
  function processHandoff(source) {
    if (claimingHandoff) return;
    var H = readHandoff();
    if (!handoffIsActionable(H)) {
      // stale cleanup
      if (H) {
        var empty =
          (!H.unfinished || !H.unfinished.length) &&
          (!H.upload_pending || !H.upload_pending.length);
        var expired = now() - Number(H.at || 0) > HANDOFF_TTL_MS;
        if (empty || expired) {
          if (!H.claimed_by || H.claimed_by === tabId) clearKey(LS_HANDOFF);
        }
      }
      if (serverHasB10() && H && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("server_has_b10");
      }
      return;
    }
    // Upload assist: any tab (light or heavy) can help drain / recollect missing
    if (H.upload_pending && H.upload_pending.length) {
      kickUploadAssist(H.upload_pending, "upload_handoff_" + (source || "poll"));
    }
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    if (!needCollect) {
      // only upload path — no heavy upgrade required
      if (serverHasB10() && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("upload_done");
      }
      return;
    }
    // If we are already heavy, just kick work and mark claim
    if (role === "heavy") {
      H.claimed_by = tabId;
      H.claimed_at = now();
      writeJson(LS_HANDOFF, H);
      kickHeavyWork("handoff_already_heavy");
      return;
    }
    claimingHandoff = true;
    // Optimistic claim stamp (helps multi-survivor race)
    H.claimed_by = tabId;
    H.claimed_at = now();
    writeJson(LS_HANDOFF, H);
    tryAcquireHeavy(source === "start" ? "handoff_resume" : "handoff").then(function (ok) {
      claimingHandoff = false;
      if (!ok) {
        // Lost race — leave ticket for winner; clear our claim if still us
        var H2 = readHandoff();
        if (H2 && H2.claimed_by === tabId) {
          H2.claimed_by = null;
          H2.claimed_at = 0;
          writeJson(LS_HANDOFF, H2);
        }
        // Still help upload as light
        kickUploadAssist((H && H.upload_pending) || [], "light_upload_assist");
        return;
      }
      stats.handoffs_claim++;
      stats.handoffs_recv++;
      timeline("handoff_claim", { source: source || "poll", unfinished: H.unfinished });
      ops(
        "gpi_handoff_claim",
        { tab: tabId, source: source || "poll", unfinished: H.unfinished },
        "warn"
      );
      kickHeavyWork("handoff_claim");
    });
  }

  function onPageHide() {
    // Best-effort flush uploads first (GA4/Sentry/PostHog pattern)
    try {
      if (global.GRUploadQueue && GRUploadQueue.flush) {
        GRUploadQueue.flush("pagehide");
      }
    } catch (eFl) {}
    var unfinished = collectUnfinished();
    var uploadPending = collectUploadPending();
    if (unfinished.length || uploadPending.length) {
      stats.handoffs_sent++;
      writeDurableHandoff(
        unfinished,
        role === "heavy" ? "pagehide_heavy" : "pagehide",
        uploadPending
      );
      broadcast({
        type: "handoff",
        unfinished: unfinished,
        upload_pending: uploadPending,
        cycle: sid(),
        role: role,
        durable: true,
      });
      timeline("handoff_send", {
        unfinished: unfinished,
        upload_pending: uploadPending,
        durable: true,
      });
      ops(
        "gpi_handoff_send",
        {
          unfinished: unfinished,
          upload_pending: uploadPending,
          tab: tabId,
          durable: true,
        },
        "warn"
      );
    }
    dropPeer();
    if (role === "heavy") releaseHeavy("pagehide");
  }

  function start(opts) {
    opts = opts || {};
    if (started) return api;
    started = true;
    tabId = mintTabId();
    try {
      global.__GR_GPI_START_MS__ = now();
    } catch (e) {}
    if (opts.cycle_id) cycleId = String(opts.cycle_id);
    try {
      cycleId = cycleId || sid();
    } catch (e2) {}

    try {
      if (typeof BroadcastChannel !== "undefined") {
        bc = new BroadcastChannel(CH_NAME);
        bc.onmessage = onMessage;
      }
    } catch (eBc) {
      bc = null;
    }

    try {
      global.addEventListener("pagehide", onPageHide);
      global.addEventListener("beforeunload", onPageHide);
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "visible") {
            heartbeat();
            maybeUpgrade();
          }
        });
      }
      global.addEventListener("storage", function (ev) {
        if (!ev) return;
        if (ev.key === LS_LEASE || ev.key === LS_PROGRESS) {
          maybeUpgrade();
        }
        if (ev.key === LS_HANDOFF) {
          processHandoff("storage");
        }
      });
    } catch (eH) {}

    // Initial role acquisition; then process any durable handoff left by a closed tab
    // or by same-tab multipage navigation (pagehide wrote ticket before unload).
    tryAcquireHeavy(opts.why || "start").then(function (ok) {
      if (!ok) setRole("light", "start_follower");
      publishRole();
      // Slight delay so self-heal/pack_loader are ready for ensureGap
      setTimeout(function () {
        processHandoff("start");
      }, 80);
    });

    hbTimer = setInterval(heartbeat, HEARTBEAT_MS);
    watchTimer = setInterval(function () {
      maybeUpgrade();
      processHandoff("watch");
      if (serverHasB10()) {
        clearHandoff("watch_server_b10");
        if (role === "heavy") {
          // Keep heavy briefly for deepen; demote after coverage stable
          writeProgress({ has_b10_server: true, phase: "b10_done" });
          broadcast({ type: "has_b10", cycle: sid() });
        }
      }
    }, 3000);

    timeline("start", { tab: tabId });
    ops("gpi_start", { tab: tabId }, "info");
    return api;
  }

  function stop(why) {
    started = false;
    if (hbTimer) {
      try {
        clearInterval(hbTimer);
      } catch (e) {}
      hbTimer = null;
    }
    if (watchTimer) {
      try {
        clearInterval(watchTimer);
      } catch (e2) {}
      watchTimer = null;
    }
    onPageHide();
    try {
      if (bc) bc.close();
    } catch (e3) {}
    bc = null;
    ops("gpi_stop", { why: why || "stop", tab: tabId }, "info");
  }

  function snapshot() {
    return {
      started: started,
      tab: tabId,
      role: role,
      heavy: role === "heavy",
      can_heavy: canHeavyCollect(),
      lock_held: lockHeld,
      lease: leaseSnapshot(),
      progress: readJson(LS_PROGRESS),
      handoff: readHandoff(),
      peers: peerCount(),
      policy: policySnapshot(),
      stats: Object.assign({}, stats),
      cycle: sid(),
      server_has_b10: serverHasB10(),
    };
  }

  var api = {
    __ready: true,
    start: start,
    stop: stop,
    tryAcquireHeavy: tryAcquireHeavy,
    releaseHeavy: releaseHeavy,
    canHeavyCollect: canHeavyCollect,
    isHeavy: isHeavy,
    getRole: function () {
      return role;
    },
    getTabId: function () {
      return tabId;
    },
    peerCount: peerCount,
    policy: policySnapshot,
    snapshot: snapshot,
    writeProgress: writeProgress,
    serverHasB10: serverHasB10,
    processHandoff: processHandoff,
    clearHandoff: clearHandoff,
    readHandoff: readHandoff,
  };

  global.GROriginCoordinator = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- probe_method_matrix.js ---- */
/**
 * GR Probe Method Matrix — multi-method recovery (not brute same-path re-kick).
 *
 * Industry rationale:
 *   Thumbmark/FPJS: per-component timeout + degrade, not infinite retry.
 *   Cao et al. / residual literature: WebGL multipath profiles are distinct signals.
 *   Gecko/WebKit need different default multipath budgets than Blink.
 *
 * Public API: GRProbeMethodMatrix
 *   - nextMethod(batchId, attempt, engine) → { method_id, profile?, flags }
 *   - isUnsupported(reason|fields) → bool
 *   - engineB10xOrder(engine) → batch id list
 *   - shouldKick(batchId, attempt, lastErr) → bool
 *
 * @see reports/FE_PROBE_CAPABILITY_LEADERSHIP_AUDIT_V1.md
 */
(function (global) {
  "use strict";

  if (global.GRProbeMethodMatrix && global.GRProbeMethodMatrix.__ready) return;

  var UNSUPPORTED_RE =
    /unsupported|no_webgl|no_multipath|no_adapter|webgpu_unavailable|not_available|capability.?missing|helpers_missing|no_sab|need_coop/i;

  /**
   * B10 recover path family (attempt is 1-based after first stall).
   * attempt 0 = initial (default multipath in pack itself).
   */
  var B10_METHODS = {
    blink: [
      { method_id: "b10_default", profile: "silicon_noderiv", multipath_cap: 5 },
      { method_id: "b10_softgl", profile: "softgl", multipath_cap: 3, gl_hard_lose: true },
      { method_id: "b10_legacy", profile: "legacy_webgl1", multipath_cap: 2 },
      { method_id: "b10_cpu_audio_degrade", profile: "cpu_audio_only", multipath_cap: 0, degrade: true },
    ],
    gecko: [
      { method_id: "b10_gecko_lite", profile: "silicon_noderiv", multipath_cap: 2 },
      { method_id: "b10_softgl", profile: "softgl", multipath_cap: 2, gl_hard_lose: true },
      { method_id: "b10_legacy", profile: "legacy_webgl1", multipath_cap: 1 },
      { method_id: "b10_cpu_audio_degrade", profile: "cpu_audio_only", multipath_cap: 0, degrade: true },
    ],
    webkit: [
      { method_id: "b10_webkit", profile: "webkit_deep", multipath_cap: 3 },
      { method_id: "b10_webkit_wave2", profile: "webkit_wave2", multipath_cap: 3 },
      { method_id: "b10_softgl", profile: "softgl", multipath_cap: 2, gl_hard_lose: true },
      { method_id: "b10_cpu_audio_degrade", profile: "cpu_audio_only", multipath_cap: 0, degrade: true },
    ],
    unknown: [
      { method_id: "b10_default", profile: "silicon_noderiv", multipath_cap: 3 },
      { method_id: "b10_webkit_safe", profile: "webkit_safe", multipath_cap: 2 },
      { method_id: "b10_legacy", profile: "legacy_webgl1", multipath_cap: 1 },
      { method_id: "b10_softgl", profile: "softgl", multipath_cap: 2, gl_hard_lose: true },
      { method_id: "b10_cpu_audio_degrade", profile: "cpu_audio_only", multipath_cap: 0, degrade: true },
    ],
  };

  /** Eager B10x order after B10 lands — engine-aware (P0). */
  var B10X_ORDER = {
    blink: [
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_silicon_ulp",
      "B10x_angle_crosscheck",
      "B10x_silicon_deep",
    ],
    gecko: [
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_softgl_hedge",
      "B10x_silicon_ulp",
      "B10x_legacy_webgl1",
    ],
    webkit: [
      "B10x_webkit_gl_noise",
      "B10x_webkit_wave2_warm4",
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_softgl_hedge",
    ],
    unknown: [
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_silicon_ulp",
      "B10x_webkit_safe",
      "B10x_legacy_webgl1",
      "B10x_unknown_kernel",
    ],
  };

  function normEngine(eng) {
    eng = String(eng || "unknown").toLowerCase();
    if (eng.indexOf("gecko") >= 0 || eng === "firefox") return "gecko";
    if (eng.indexOf("webkit") >= 0 || eng === "safari") return "webkit";
    if (eng.indexOf("blink") >= 0 || eng === "chrome" || eng === "edge" || eng === "opera") return "blink";
    return B10_METHODS[eng] ? eng : "unknown";
  }

  /**
   * Return capability evidence, not a browser-name guess. A single API is
   * intentionally insufficient because wrappers and compatibility layers may
   * expose another engine's surface. UA/brands stay claims and are not used
   * here.
   */
  function engineEvidence() {
    var scores = { blink: 0, gecko: 0, webkit: 0 };
    var signals = { blink: [], gecko: [], webkit: [] };
    function add(engine, signal, ok) {
      if (ok) {
        scores[engine]++;
        signals[engine].push(signal);
      }
    }
    try {
      var nav = global.navigator || {};
      var css = global.CSS;
      add("gecko", "moz_inner_screen", typeof global.mozInnerScreenX === "number");
      add("gecko", "moz_css", !!(css && css.supports && css.supports("-moz-appearance", "none")));
      add("blink", "chrome_runtime", !!(global.chrome && global.chrome.runtime));
      add("blink", "user_agent_data", !!nav.userAgentData);
      add("webkit", "webkit_audio", !!(global.webkitAudioContext || global.AudioContext && nav.vendor === "Apple Computer, Inc."));
      add("webkit", "webkit_css", !!(css && css.supports && css.supports("-webkit-touch-callout", "none")));
    } catch (e) {}
    var best = "unknown";
    var bestScore = 1;
    Object.keys(scores).forEach(function (engine) {
      if (scores[engine] > bestScore) {
        best = engine;
        bestScore = scores[engine];
      }
    });
    return { classified: best, confidence: Math.min(1, bestScore / 3), scores: scores, signals: signals };
  }

  function detectEngine() {
    return engineEvidence().classified;
  }

  function methodsFor(batchId, engine) {
    var eng = normEngine(engine || detectEngine());
    var bid = String(batchId || "");
    if (bid === "B10_hw_curves" || bid === "mid.curves") {
      return (B10_METHODS[eng] || B10_METHODS.unknown).slice();
    }
    if (bid.indexOf("B10x_") === 0) {
      // Single-method packs; rotate only softgl / legacy on retry
      return [
        { method_id: "b10x_native", profile: bid, multipath_cap: eng === "gecko" ? 2 : 3 },
        { method_id: "b10x_compat", profile: "compat_surface", multipath_cap: 2 },
        { method_id: "b10x_softgl", profile: "softgl", multipath_cap: 2, gl_hard_lose: true },
        { method_id: "b10x_legacy", profile: "legacy_webgl1", multipath_cap: 1 },
        { method_id: "b10x_skip", profile: "skip", multipath_cap: 0, honest_skip: true },
      ];
    }
    if (bid === "B18_webgpu") {
      return [
        { method_id: "webgpu_compute", profile: "compute_residual" },
        { method_id: "webgpu_adapter_only", profile: "adapter_meta" },
        { method_id: "webgpu_skip", profile: "skip", honest_skip: true },
      ];
    }
    if (bid === "B46_audio_deep") {
      return [
        { method_id: "audio_oac", profile: "offline_audio" },
        { method_id: "audio_worklet", profile: "worklet" },
        { method_id: "audio_skip", profile: "skip", honest_skip: true },
      ];
    }
    // Generic soft: one retry then honest skip
    return [
      { method_id: "default", profile: "default" },
      { method_id: "skip", profile: "skip", honest_skip: true },
    ];
  }

  /**
   * @param {string} batchId
   * @param {number} attempt 0-based recover index (0 = first stall recovery → method[1] or [0])
   * @param {string} [engine]
   */
  function nextMethod(batchId, attempt, engine) {
    var list = methodsFor(batchId, engine);
    var i = Math.max(0, Number(attempt) || 0);
    // On recoveries, advance method: recovery #1 → index 1 if exists else 0
    var idx = Math.min(i, list.length - 1);
    // If still on first kick, packs use their internal default; recover starts at 0 → use method 0 after lose
    var m = list[idx] || list[list.length - 1] || { method_id: "default", profile: "default" };
    return Object.assign({ engine: normEngine(engine || detectEngine()), method_index: idx, methods_n: list.length }, m);
  }

  function isUnsupported(reasonOrFields) {
    if (reasonOrFields == null) return false;
    if (typeof reasonOrFields === "string") return UNSUPPORTED_RE.test(reasonOrFields);
    try {
      var f = reasonOrFields;
      if (f.b10x_capability === "unsupported") return true;
      if (f.webgpu_available === false && f.b18_ok === false) return true;
      var err = String(f.b10x_err || f.error || f.err || f.skip_reason || f.webgpu_err || "");
      if (UNSUPPORTED_RE.test(err)) return true;
      if (f.webgl_support === false && (f.b10x_ok === false || f.residual_ok === false)) return true;
    } catch (e) {}
    return false;
  }

  function engineB10xOrder(engine) {
    var eng = normEngine(engine || detectEngine());
    return (B10X_ORDER[eng] || B10X_ORDER.unknown).slice();
  }

  /**
   * Whether self-heal may re-kick this batch.
   * @param {string} batchId
   * @param {number} attempt collect_attempts so far
   * @param {string} [lastErr]
   * @param {object} [method] last method chosen
   */
  function shouldKick(batchId, attempt, lastErr, method) {
    if (isUnsupported(lastErr)) return false;
    if (method && method.honest_skip) return false;
    if (method && method.degrade && Number(attempt) > 0) {
      // allow one degrade path collect, then stop thrash
      return Number(attempt) <= 1;
    }
    var list = methodsFor(batchId);
    // Cap kicks at methods length + 1 (initial)
    if (Number(attempt) >= list.length + 1) return false;
    return true;
  }

  /**
   * Apply method hints onto global so multipath / packs can read them.
   */
  function publishMethod(batchId, method) {
    try {
      global.__GR_PROBE_METHOD__ = global.__GR_PROBE_METHOD__ || {};
      global.__GR_PROBE_METHOD__[String(batchId || "")] = method;
      global.__GR_PROBE_METHOD_LAST__ = method;
      if (method && method.multipath_cap != null) {
        global.__GR_MULTIPATH_CAP_HINT__ = method.multipath_cap;
      }
      if (method && method.profile) {
        global.__GR_PROBE_PROFILE_HINT__ = method.profile;
      }
    } catch (e) {}
    return method;
  }

  function snapshot() {
    return {
      engine: detectEngine(),
      engine_evidence: engineEvidence(),
      last: global.__GR_PROBE_METHOD_LAST__ || null,
      b10x_order: engineB10xOrder(),
      methods_b10: methodsFor("B10_hw_curves"),
    };
  }

  var api = {
    __ready: true,
    nextMethod: nextMethod,
    methodsFor: methodsFor,
    isUnsupported: isUnsupported,
    engineB10xOrder: engineB10xOrder,
    shouldKick: shouldKick,
    publishMethod: publishMethod,
    detectEngine: detectEngine,
    snapshot: snapshot,
  };

  global.GRProbeMethodMatrix = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- probe_self_heal.js ---- */
/**
 * greenpng Probe Self-Heal — Gap Table + Supervisor + loops T/C/B + adaptive.
 *
 * Authority:
 *   Server brain (route_plan / coverage / cool) decides desired + terminal.
 *   FE only gap-fills: Transport (T), Collect (C), Brain reconcile (B).
 *
 * Smart FE (not dumb probe/upload):
 *   - Engine-aware adaptive policy (blink / gecko / webkit)
 *   - Stall detection + recovery (soft stall → hard recover → circuit break)
 *   - Priority gate: never thrash B10x/deepen while primary B10 unsatisfied
 *   - Timeline ring for diagnostics (ops + __GR_PROBE_TIMELINE__)
 *
 * Completion:
 *   Batch done ⇔ material acked OR server received — NEVER only "kicked".
 *
 * Scope:
 *   Per page / origin. No cross-site serial lock (browser isolation).
 *   Same-site sticky cycle resume is handled by storage + open; this module
 *   keeps healing while the page stays open.
 */
(function (global) {
  "use strict";

  if (global.GRProbeSelfHeal && global.GRProbeSelfHeal.__ready) return;

  var HARD_DEFAULT = [
    "B10_hw_curves",
    "B0_bootstrap",
    "B2_hardware",
    "B3_system",
    "B1_conflict",
    "B12_anti_camouflage",
    "B8_gateway",
    "B8_gateway_early",
    "B7_sandbox",
  ];

  var CFG = {
    /** Supervisor base tick (ms). Visible tabs tick faster. */
    tick_hidden_ms: 8000,
    tick_visible_ms: 2500,
    /** Loop B poll spacing (ms). */
    brain_poll_visible_ms: 4000,
    brain_poll_hidden_ms: 20000,
    brain_poll_gap_ms: 2500,
    /** Loop C: B10 run timeout (ms) before collect miss / re-kick. */
    // Hard ceiling; soft stall fires earlier via adaptive policy.
    b10_run_timeout_ms: 90000,
    b10x_run_timeout_ms: 30000,
    heavy_run_timeout_ms: 45000,
    /** Soft stall: recover before hard timeout (ms). Engine policy may override. */
    b10_soft_stall_ms: 25000,
    heavy_soft_stall_ms: 18000,
    /** Loop C: long-window recollect delays when still missing after kick (ms). */
    collect_watchdog_ms: [800, 2500, 6000, 15000, 30000, 60000, 120000, 180000],
    /** Max collect re-kicks per batch per session (hard). Soft packs lower. */
    collect_max_attempts: 10,
    soft_collect_max_attempts: 3,
    deepen_collect_max_attempts: 4,
    /** Max brain applyRoutePlan kicks per plan_epoch. */
    brain_apply_budget: 12,
    /** Min gap between same-batch collect re-kicks. */
    collect_min_gap_ms: 4000,
    /** Transport nudge min gap. */
    transport_nudge_gap_ms: 2000,
    /** Adaptive mode: auto | aggressive | balanced | conservative | recovery */
    adaptive_mode: "auto",
    /** Timeline ring size. */
    timeline_max: 80,
    /** Circuit: consecutive soft-pack fails before pause deepen (ms quiet). */
    deepen_circuit_fails: 4,
    deepen_circuit_quiet_ms: 20000,
  };

  /** @type {Object.<string, GapRec>} */
  var gaps = Object.create(null);
  var started = false;
  var timer = null;
  var lastBrainPollMs = 0;
  var lastBrainRev = 0;
  var brainApplyN = 0;
  var lastPlanEpoch = null;
  var eventTickTimer = null;
  var eventListenersBound = false;
  var timeline = [];
  var adaptive = {
    mode: "balanced",
    engine: "unknown",
    max_light: 2,
    serial_heavy: false,
    pause_deepen_until: 0,
    stall_recoveries: 0,
    last_stall_ms: 0,
    last_policy_ms: 0,
  };
  var stats = {
    ticks: 0,
    t_nudges: 0,
    c_kicks: 0,
    method_switches: 0,
    unsupported_stops: 0,
    b_applies: 0,
    b_polls: 0,
    satisfied: 0,
    stalls: 0,
    stall_recoveries: 0,
    circuit_trips: 0,
    deepen_skipped: 0,
  };

  /**
   * @typedef {object} GapRec
   * @property {string} batch_id
   * @property {string} source
   * @property {boolean} desired
   * @property {boolean} server_seen
   * @property {boolean} force_recollect
   * @property {number} generation
   * @property {number} collect_attempts
   * @property {number} transport_nudges
   * @property {number} last_collect_ms
   * @property {number} last_transport_ms
   * @property {number} run_started_ms
   * @property {string} local_phase
   * @property {number} priority
   * @property {string} reason
   */

  function now() {
    return Date.now();
  }

  function sid() {
    try {
      return String(
        global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.GRUploadQueue &&
            GRUploadQueue.cfg &&
            GRUploadQueue.cfg.session_id) ||
          ""
      );
    } catch (e) {
      return "";
    }
  }

  function pageCool() {
    try {
      if (global.__GR_BRAIN_SCHEDULE_FINAL__ || global.__GR_HARD_FINAL__) return true;
      if (global.__GR_SKIP_IDENTITY__ && global.__GR_STOP_PROBE__) return true;
      if (global.__GR_PHASE__ === "cool" && global.__GR_HARD_FINAL__) return true;
      if (
        global.GRProbeLifecycle &&
        GRProbeLifecycle.snapshot &&
        GRProbeLifecycle.snapshot().state === "complete_cool"
      ) {
        return true;
      }
    } catch (e) {}
    return false;
  }

  function pageUnloading() {
    try {
      return !!(
        global.__GR_PAGE_UNLOADING__ ||
        (global.GRUploadQueue &&
          GRUploadQueue.isPageUnloading &&
          GRUploadQueue.isPageUnloading())
      );
    } catch (e) {
      return false;
    }
  }

  function visible() {
    try {
      return !(typeof document !== "undefined" && document.visibilityState === "hidden");
    } catch (e) {
      return true;
    }
  }

  function isHard(bid) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isHardBatch) {
        return !!GRProbeLifecycle.isHardBatch(bid);
      }
    } catch (e) {}
    var id = String(bid || "");
    if (id.indexOf("B10x_") === 0) return false;
    for (var i = 0; i < HARD_DEFAULT.length; i++) if (HARD_DEFAULT[i] === id) return true;
    return id === "B10_hw_curves" || id === "mid.curves";
  }

  function isB10(bid) {
    return String(bid || "") === "B10_hw_curves" || String(bid || "") === "mid.curves";
  }

  function isB10x(bid) {
    return String(bid || "").indexOf("B10x_") === 0;
  }

  function gapKey(batchId, source) {
    return String(batchId || "") + "|" + String(source || "main");
  }

  function ensureGap(batchId, source, patch) {
    var bid = String(batchId || "");
    if (!bid) return null;
    var src = String(source || "main");
    var k = gapKey(bid, src);
    var g = gaps[k];
    if (!g) {
      g = {
        batch_id: bid,
        source: src,
        desired: true,
        server_seen: false,
        force_recollect: false,
        generation: 1,
        collect_attempts: 0,
        transport_nudges: 0,
        last_collect_ms: 0,
        last_transport_ms: 0,
        run_started_ms: 0,
        local_phase: "absent",
        priority: isB10(bid) ? 1000 : isHard(bid) ? 800 : 100,
        reason: "seed",
      };
      gaps[k] = g;
    }
    if (patch && typeof patch === "object") {
      Object.keys(patch).forEach(function (pk) {
        g[pk] = patch[pk];
      });
    }
    return g;
  }

  function debugOn() {
    try {
      return !!(
        global.__GR_DEBUG_GPI__ ||
        global.__GR_DEBUG__ ||
        (global.__GR_BOOT__ && (global.__GR_BOOT__.debug_gpi || global.__GR_BOOT__.debug))
      );
    } catch (e) {
      return false;
    }
  }

  function clog(level, msg, detail) {
    if (!debugOn()) return;
    try {
      var fn = (global.console && (global.console[level] || global.console.log)) || null;
      if (!fn) return;
      var line = "[gr:heal] " + msg;
      if (detail != null) fn.call(global.console, line, detail);
      else fn.call(global.console, line);
    } catch (e) {}
  }

  function ops(code, stage, detail, sev) {
    try {
      if (global.GROps && GROps.report) {
        GROps.report(code, stage || "self_heal", detail || {}, sev || "info");
      }
    } catch (e) {}
    if (
      debugOn() &&
      (sev === "warn" ||
        sev === "error" ||
        /stall|timeout|circuit|skip_collect|gap_c_|gpi_/i.test(String(code || "")))
    ) {
      clog(sev === "error" ? "error" : "warn", String(code), detail || null);
    }
  }

  function timelinePush(ev, detail) {
    try {
      var row = {
        t: now(),
        ev: String(ev || ""),
        d: detail || null,
      };
      timeline.push(row);
      if (timeline.length > (CFG.timeline_max || 80)) {
        timeline = timeline.slice(-(CFG.timeline_max || 80));
      }
      global.__GR_PROBE_TIMELINE__ = timeline;
    } catch (e) {}
  }

  /**
   * Lightweight engine family (capability-first; not pure UA trust).
   * Peer products (FPJS/BotD) also branch on gecko vs blink for audio/webgl quirks.
   */
  function detectEngine() {
    try {
      if (global.GROps && typeof GROps.engineFamily === "function") {
        return String(GROps.engineFamily() || "unknown");
      }
    } catch (e0) {}
    try {
      var ua = String((global.navigator && navigator.userAgent) || "");
      if (typeof global.mozInnerScreenX === "number") return "gecko";
      if (/Firefox\//.test(ua) || (/Gecko\//.test(ua) && !/like Gecko/.test(ua) && !/Chrome\//.test(ua)))
        return "gecko";
      if (
        /AppleWebKit\//.test(ua) &&
        /Safari\//.test(ua) &&
        !/Chrome\/|Chromium\/|Edg\/|OPR\/|CriOS\//.test(ua)
      )
        return "webkit";
      if (/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua)) return "blink";
    } catch (e1) {}
    return "unknown";
  }

  function publishAdaptivePolicy() {
    try {
      global.__GR_ADAPTIVE_POLICY__ = {
        mode: adaptive.mode,
        engine: adaptive.engine,
        max_light: adaptive.max_light,
        serial_heavy: !!adaptive.serial_heavy,
        pause_deepen_until: adaptive.pause_deepen_until || 0,
        b10_soft_stall_ms: CFG.b10_soft_stall_ms,
        b10_run_timeout_ms: CFG.b10_run_timeout_ms,
        collect_min_gap_ms: CFG.collect_min_gap_ms,
      };
      if (adaptive.serial_heavy) {
        global.__GR_PACK_MAX_CONCURRENT__ = 1;
        global.__GR_PACK_MAX_LIGHT__ = 1;
      } else {
        global.__GR_PACK_MAX_LIGHT__ = adaptive.max_light;
        if (global.__GR_PACK_MAX_CONCURRENT__ === 1 && adaptive.mode !== "recovery") {
          try {
            delete global.__GR_PACK_MAX_CONCURRENT__;
          } catch (eD) {
            global.__GR_PACK_MAX_CONCURRENT__ = 0;
          }
        }
      }
    } catch (eP) {}
  }

  /**
   * Apply engine + runtime pressure policy. Smarter than fixed timers:
   * gecko/webkit: fewer concurrent light packs, earlier soft stall, longer min kick gap.
   * recovery: serial heavy only, deepen paused.
   */
  function applyAdaptivePolicy(force) {
    var t = now();
    if (!force && adaptive.last_policy_ms && t - adaptive.last_policy_ms < 3000) return adaptive;
    adaptive.last_policy_ms = t;
    adaptive.engine = detectEngine();
    var mode = CFG.adaptive_mode || "auto";
    if (mode === "auto") {
      if (adaptive.stall_recoveries >= 2 || adaptive.pause_deepen_until > t) mode = "recovery";
      else if (adaptive.engine === "gecko" || adaptive.engine === "webkit") mode = "conservative";
      else mode = "balanced";
    }
    adaptive.mode = mode;

    if (mode === "aggressive") {
      adaptive.max_light = 2;
      adaptive.serial_heavy = false;
      CFG.b10_soft_stall_ms = 35000;
      CFG.b10_run_timeout_ms = 90000;
      CFG.collect_min_gap_ms = 3000;
      CFG.tick_visible_ms = 2000;
    } else if (mode === "conservative") {
      adaptive.max_light = 1;
      adaptive.serial_heavy = true;
      CFG.b10_soft_stall_ms = 20000;
      CFG.b10_run_timeout_ms = 75000;
      CFG.b10x_run_timeout_ms = 25000;
      CFG.collect_min_gap_ms = 6000;
      CFG.tick_visible_ms = 3000;
      CFG.soft_collect_max_attempts = 2;
      CFG.deepen_collect_max_attempts = 3;
    } else if (mode === "recovery") {
      adaptive.max_light = 1;
      adaptive.serial_heavy = true;
      CFG.b10_soft_stall_ms = 15000;
      CFG.b10_run_timeout_ms = 60000;
      CFG.collect_min_gap_ms = 8000;
      CFG.tick_visible_ms = 3500;
      // Only hard anchors while recovering.
      adaptive.pause_deepen_until = Math.max(adaptive.pause_deepen_until || 0, t + 15000);
    } else {
      // balanced
      adaptive.max_light = adaptive.engine === "blink" ? 2 : 1;
      adaptive.serial_heavy = adaptive.engine !== "blink";
      CFG.b10_soft_stall_ms = 25000;
      CFG.b10_run_timeout_ms = 90000;
      CFG.collect_min_gap_ms = 4000;
      CFG.tick_visible_ms = 2500;
    }

    // Lab multi-tab pressure hint
    try {
      if (global.__GR_LAB_PRESSURE__) {
        adaptive.max_light = 1;
        adaptive.serial_heavy = true;
      }
    } catch (eL) {}

    // GPI multi-tab brain linkage (origin_coordinator policy)
    // More same-origin tabs ⇒ heavy serializes silicon; light focuses transport.
    try {
      var gpiPol =
        (global.GROriginCoordinator &&
          GROriginCoordinator.policy &&
          GROriginCoordinator.policy()) ||
        global.__GR_GPI_POLICY__ ||
        null;
      var peers =
        (gpiPol && gpiPol.peer_count) ||
        Number(global.__GR_GPI_PEER_COUNT__ || 0) ||
        0;
      if (peers > 1 || (gpiPol && gpiPol.multi_tab)) {
        adaptive.multi_tab = true;
        adaptive.peer_count = peers;
        if (gpiPol && gpiPol.defer_heavy_collect) {
          // light tab: no deepen thrash; transport/RPA only
          adaptive.max_light = 1;
          adaptive.serial_heavy = true;
          adaptive.pause_deepen_until = Math.max(
            adaptive.pause_deepen_until || 0,
            t + 8000
          );
          CFG.collect_min_gap_ms = Math.max(CFG.collect_min_gap_ms || 4000, 6000);
          CFG.transport_nudge_gap_ms = Math.min(CFG.transport_nudge_gap_ms || 2000, 1200);
        } else if (gpiPol && gpiPol.serial_heavy) {
          // heavy under multi-tab: finish B10 first, less concurrent light packs
          adaptive.max_light = 1;
          adaptive.serial_heavy = true;
          CFG.collect_min_gap_ms = Math.max(CFG.collect_min_gap_ms || 4000, 5000);
        }
      } else {
        adaptive.multi_tab = false;
        adaptive.peer_count = peers || 1;
      }
    } catch (eG) {}

    publishAdaptivePolicy();
    timelinePush("adaptive_policy", {
      mode: adaptive.mode,
      engine: adaptive.engine,
      max_light: adaptive.max_light,
      serial: adaptive.serial_heavy,
      multi_tab: !!adaptive.multi_tab,
      peers: adaptive.peer_count || 0,
    });
    return adaptive;
  }

  function isDeepenBatch(bid) {
    var id = String(bid || "");
    if (id.indexOf("B10x_") === 0) return true;
    if (id === "B18_webgpu" || id === "B46_audio_deep" || id === "B47_sab_clock") return true;
    if (id.indexOf("R") === 0 && /_spotcheck$/.test(id)) return true;
    return false;
  }

  function isSoftBatch(bid) {
    if (isHard(bid) || isB10(bid)) return false;
    return !isDeepenBatch(bid);
  }

  /** Origin multi-tab: only heavy role may collect hard/B10 (GPI). */
  function mayHeavyCollect() {
    try {
      if (!(global.GROriginCoordinator && GROriginCoordinator.__ready)) return true;
      if (GROriginCoordinator.isHeavy && GROriginCoordinator.isHeavy()) return true;
      var snap = GROriginCoordinator.snapshot && GROriginCoordinator.snapshot();
      // Coordinator not started yet / sole tab with no foreign lease → allow heavy
      if (!snap || !snap.started) return true;
      var L = snap.lease;
      if (!L || !L.tab) return true;
      if (L.tab === snap.tab) return true;
      var age = now() - Number(L.hb || L.at || 0);
      // Peer leader heartbeat fresh → we are light follower
      if (age < 6000) return false;
      // Stale peer lease → allow (upgrade path)
      return true;
    } catch (e) {
      return true;
    }
  }

  /** While primary B10 missing: only hard anchors + transport — no deepen thrash. */
  function shouldSkipCollect(bid) {
    // Multi-tab GPI: light tabs skip all hard silicon re-collect
    if ((isB10(bid) || isHard(bid)) && !mayHeavyCollect()) {
      stats.deepen_skipped++;
      timelinePush("skip_collect", { batch_id: bid, why: "gpi_not_heavy" });
      return true;
    }
    if (isB10(bid) || isHard(bid)) return false;
    var t = now();
    if (adaptive.pause_deepen_until && t < adaptive.pause_deepen_until) {
      if (isDeepenBatch(bid) || isSoftBatch(bid)) {
        stats.deepen_skipped++;
        return true;
      }
    }
    // Priority gate: B10 still desired & unsatisfied → skip deepen re-collect.
    try {
      var gb = ensureGap("B10_hw_curves", "main");
      if (gb && gb.desired && !isSatisfied(gb)) {
        if (isDeepenBatch(bid)) {
          stats.deepen_skipped++;
          return true;
        }
      }
    } catch (e) {}
    // Deepen only on heavy tab (save GPU for leader)
    if (isDeepenBatch(bid) && !mayHeavyCollect()) {
      stats.deepen_skipped++;
      return true;
    }
    return false;
  }

  function softStallMs(bid) {
    if (isB10(bid)) return CFG.b10_soft_stall_ms || 25000;
    if (isB10x(bid) || isDeepenBatch(bid)) return Math.min(CFG.heavy_soft_stall_ms || 18000, 20000);
    return CFG.heavy_soft_stall_ms || 18000;
  }

  /**
   * Stall recovery: free GPU/bus, then **switch probe method** (not same-path thrash).
   * Method order from GRProbeMethodMatrix (engine-aware). Industry: Thumbmark timeout
   * degrade + multipath literature — change profile before re-kick.
   */
  function recoverFromStall(g, why) {
    if (!g) return false;
    var t = now();
    // Debounce recoveries
    if (adaptive.last_stall_ms && t - adaptive.last_stall_ms < 4000) return false;
    adaptive.last_stall_ms = t;
    adaptive.stall_recoveries = (adaptive.stall_recoveries || 0) + 1;
    stats.stalls++;
    stats.stall_recoveries++;

    // --- Method rotation (P0) ---
    var method = null;
    try {
      var MM = global.GRProbeMethodMatrix;
      if (MM && MM.__ready) {
        var eng = adaptive.engine || (MM.detectEngine && MM.detectEngine()) || "unknown";
        var mIdx = Math.max(0, (g.method_switches || 0));
        method = MM.nextMethod(g.batch_id, mIdx, eng);
        g.method_switches = (g.method_switches || 0) + 1;
        g.last_method = method;
        if (MM.publishMethod) MM.publishMethod(g.batch_id, method);
        stats.method_switches = (stats.method_switches || 0) + 1;
        // Unsupported / honest skip → do NOT re-kick
        if (method.honest_skip || method.degrade === true && g.method_switches > 2) {
          g.local_phase = method.honest_skip ? "honest_skip" : "degrade_done";
          g.terminal_local = method.honest_skip ? "skipped_method" : "degrade";
          timelinePush("method_stop", {
            batch_id: g.batch_id,
            method: method.method_id,
            why: why || "stall",
          });
          ops(
            "probe_method_stop",
            "self_heal",
            {
              batch_id: g.batch_id,
              method: method.method_id,
              profile: method.profile,
              session_id: sid(),
            },
            "warn"
          );
          if (isDeepenBatch(g.batch_id) || isSoftBatch(g.batch_id)) {
            adaptive.pause_deepen_until = t + (CFG.deepen_circuit_quiet_ms || 20000);
          }
          return false;
        }
        if (MM.isUnsupported && MM.isUnsupported(g.last_err || why || "")) {
          g.local_phase = "unsupported";
          g.terminal_local = "unsupported";
          stats.unsupported_stops = (stats.unsupported_stops || 0) + 1;
          timelinePush("unsupported_stop", { batch_id: g.batch_id, err: g.last_err || why });
          return false;
        }
      }
    } catch (eM) {}

    timelinePush("stall_recover", {
      batch_id: g.batch_id,
      why: why || "stall",
      phase: g.local_phase,
      engine: adaptive.engine,
      n: adaptive.stall_recoveries,
      method: method && method.method_id,
      profile: method && method.profile,
    });

    clearKicked(g.batch_id);
    g.run_started_ms = 0;
    g.local_phase = "stall_recover";

    try {
      var doHardLose = !method || method.gl_hard_lose !== false;
      if (method && method.gl_hard_lose === false) doHardLose = false;
      if (doHardLose && global.GRGlGovernor && typeof GRGlGovernor.forceHardLoseForRetry === "function") {
        GRGlGovernor.forceHardLoseForRetry(
          "stall:" + String(g.batch_id || "") + ":" + String((method && method.method_id) || why || "")
        );
      }
    } catch (eGl) {}
    try {
      if (global.GRPackLoader && typeof GRPackLoader.forceReleaseResource === "function") {
        ["gpu", "audio", "cpu"].forEach(function (c) {
          GRPackLoader.forceReleaseResource(c, "stall:" + String(g.batch_id || ""));
        });
      }
    } catch (eBus) {}

    // Trip deepen circuit on repeated stalls
    if (adaptive.stall_recoveries >= 2) {
      adaptive.pause_deepen_until = t + (CFG.deepen_circuit_quiet_ms || 20000);
      stats.circuit_trips++;
      timelinePush("circuit_deepen_pause", { until: adaptive.pause_deepen_until });
      ops(
        "probe_circuit_deepen",
        "self_heal",
        { until: adaptive.pause_deepen_until, stalls: adaptive.stall_recoveries, session_id: sid() },
        "warn"
      );
    }
    applyAdaptivePolicy(true);

    ops(
      "probe_stall_recover",
      "self_heal",
      {
        batch_id: g.batch_id,
        why: why || "stall",
        engine: adaptive.engine,
        mode: adaptive.mode,
        recoveries: adaptive.stall_recoveries,
        method: method && method.method_id,
        profile: method && method.profile,
        multipath_cap: method && method.multipath_cap,
        session_id: sid(),
      },
      "warn"
    );

    // Re-kick only if primary / hard and method matrix allows
    if (isB10(g.batch_id) || isHard(g.batch_id)) {
      try {
        var MM2 = global.GRProbeMethodMatrix;
        if (MM2 && MM2.shouldKick && !MM2.shouldKick(g.batch_id, g.collect_attempts, g.last_err, method)) {
          g.local_phase = "method_budget_exhausted";
          return false;
        }
      } catch (eSk) {}
      g.force_recollect = true;
      return kickCollect(g, "stall_recover:" + String((method && method.method_id) || why || "stall"));
    }
    return true;
  }

  function materialState(bid) {
    try {
      if (global.GRUploadQueue && GRUploadQueue.materialState) {
        return String(GRUploadQueue.materialState(bid, sid(), "main") || "absent");
      }
    } catch (e) {}
    return "absent";
  }

  function hasPending(bid) {
    try {
      if (global.GRUploadQueue && GRUploadQueue.hasPendingCapture) {
        return !!GRUploadQueue.hasPendingCapture(bid, sid());
      }
    } catch (e) {}
    return false;
  }

  function alreadyKicked(bid) {
    try {
      if (global.GRPackLoader && GRPackLoader.alreadyKicked) {
        var m = GRPackLoader.alreadyKicked() || {};
        return !!(m[bid] || m[String(bid)]);
      }
    } catch (e) {}
    return false;
  }

  function packHealth(bid) {
    try {
      if (global.GRPackLoader && GRPackLoader.health) {
        var h = GRPackLoader.health() || {};
        return h[bid] || h[String(bid)] || null;
      }
    } catch (e) {}
    return null;
  }

  function isRunning(bid) {
    var h = packHealth(bid);
    if (h && (h.run === "running" || h.run === "pending" || h.kick === "started")) {
      var age = h.ts ? now() - Number(h.ts) : 0;
      var staleMs = (softStallMs(bid) || 20000) + 2500;
      if (h.ts && age > staleMs) {
        try {
          if (global.GRPackLoader && GRPackLoader.forceReleaseResource) {
            ["gpu", "audio", "cpu"].forEach(function (c) {
              GRPackLoader.forceReleaseResource(c, "stale_running:" + String(bid));
            });
          }
        } catch (eRel) {}
        try {
          h.run = "stale_timeout";
        } catch (eH) {}
        return false;
      }
      return true;
    }
    try {
      var locks = global.__GR_RESOURCE_LOCKS__ || {};
      var gpu = locks.gpu;
      if (gpu && String(gpu.label || "") === String(bid)) return true;
    } catch (e) {}
    return false;
  }

  function serverHasBatch(bid) {
    try {
      var rec = global.__GR_RECEIVED_BATCHES__ || [];
      for (var i = 0; i < rec.length; i++) {
        if (String(rec[i].batch_id || "") === String(bid)) return true;
      }
    } catch (e) {}
    try {
      var la = global.__GR_LAST_ANALYZE__;
      var res = (la && (la.result || la)) || {};
      var cov = res.coverage || {};
      var cps = res.cycle_probe_status || {};
      if (isB10(bid) && (cov.has_b10 === true || cps.has_b10 === true || res.b10_present === true)) {
        return true;
      }
      var ids = cov.received_batch_ids || cps.received_batch_ids || res.received_batch_ids;
      if (Array.isArray(ids)) {
        for (var j = 0; j < ids.length; j++) if (String(ids[j]) === String(bid)) return true;
      }
    } catch (e2) {}
    return false;
  }

  function isSatisfied(g) {
    if (!g) return true;
    if (g.server_seen) return true;
    var st = materialState(g.batch_id);
    if (st === "acked") return true;
    if (serverHasBatch(g.batch_id)) {
      g.server_seen = true;
      return true;
    }
    return false;
  }

  function refreshLocalPhase(g) {
    if (isSatisfied(g)) {
      g.local_phase = "acked";
      return g.local_phase;
    }
    var st = materialState(g.batch_id);
    if (st === "uploading" || st === "queued" || st === "retry_wait") {
      g.local_phase = st;
      return st;
    }
    if (st === "collected" || hasPending(g.batch_id)) {
      g.local_phase = "collected";
      return "collected";
    }
    if (isRunning(g.batch_id)) {
      g.local_phase = "running";
      if (!g.run_started_ms) g.run_started_ms = now();
      return "running";
    }
    if (alreadyKicked(g.batch_id) && st === "absent" && !hasPending(g.batch_id)) {
      // Collect claimed success but no material — treat as miss for Loop C.
      g.local_phase = "collect_orphan";
      return "collect_orphan";
    }
    if (alreadyKicked(g.batch_id)) {
      g.local_phase = "kicked";
      return "kicked";
    }
    g.local_phase = "absent";
    return "absent";
  }

  // ---------- Loop T: transport ----------
  function loopT(g) {
    if (!g || !g.desired || isSatisfied(g)) return;
    var phase = refreshLocalPhase(g);
    if (
      phase !== "queued" &&
      phase !== "uploading" &&
      phase !== "retry_wait" &&
      phase !== "collected"
    ) {
      return;
    }
    var t = now();
    if (g.last_transport_ms && t - g.last_transport_ms < CFG.transport_nudge_gap_ms) return;
    g.last_transport_ms = t;
    g.transport_nudges++;
    stats.t_nudges++;
    try {
      var Q = global.GRUploadQueue;
      if (!Q) return;
      if (typeof Q.nudgeTransport === "function") {
        Q.nudgeTransport(g.batch_id, sid());
      } else if (typeof Q.flush === "function") {
        Q.flush("self_heal_t");
      }
      ops(
        "gap_t_retry",
        "self_heal",
        {
          batch_id: g.batch_id,
          phase: phase,
          nudges: g.transport_nudges,
          session_id: sid(),
        },
        "info"
      );
    } catch (e) {}
  }

  // ---------- Loop C: collect ----------
  function runTimeoutMs(bid) {
    if (isB10(bid)) return CFG.b10_run_timeout_ms;
    if (isB10x(bid)) return CFG.b10x_run_timeout_ms;
    if (isHard(bid)) return CFG.heavy_run_timeout_ms;
    return CFG.heavy_run_timeout_ms;
  }

  function collectMax(bid) {
    var base = CFG.collect_max_attempts;
    if (isDeepenBatch(bid)) base = CFG.deepen_collect_max_attempts || 4;
    else if (isSoftBatch(bid)) base = CFG.soft_collect_max_attempts || 3;
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.maxAttemptsFor) {
        var life = GRProbeLifecycle.maxAttemptsFor(bid) | 0;
        if (isB10(bid) || isHard(bid)) return Math.max(base, life);
        return Math.min(base, life || base);
      }
    } catch (e) {}
    return base;
  }

  function watchdogReady(g) {
    var delays = CFG.collect_watchdog_ms;
    var idx = Math.min(delays.length - 1, Math.max(0, g.collect_attempts));
    var need = delays[idx] || 30000;
    if (!g.last_collect_ms && g.collect_attempts === 0) {
      // First watchdog: use first delay from session open-ish seed time.
      return true;
    }
    // PostHog-style jitter (±20%) so multi-tab / multi-browser same-host
    // collect re-kicks do not thrash GPU/ResourceBus in lockstep.
    if (g._watchdog_need_ms == null || g._watchdog_for_attempt !== g.collect_attempts) {
      var jitter = need * (0.8 + Math.random() * 0.4);
      g._watchdog_need_ms = Math.floor(jitter);
      g._watchdog_for_attempt = g.collect_attempts;
    }
    return now() - (g.last_collect_ms || 0) >= (g._watchdog_need_ms || need);
  }

  function clearKicked(bid) {
    try {
      if (global.GRPackLoader && GRPackLoader.clearKicked) {
        GRPackLoader.clearKicked(bid);
      }
    } catch (e) {}
  }

  function kickCollect(g, why) {
    if (!g || pageCool() || pageUnloading()) return false;
    if (shouldSkipCollect(g.batch_id)) {
      timelinePush("skip_collect", { batch_id: g.batch_id, why: "priority_or_circuit" });
      return false;
    }
    // Never re-kick batches marked unsupported / honest_skip
    if (g.terminal_local === "unsupported" || g.terminal_local === "skipped_method") {
      timelinePush("skip_collect", { batch_id: g.batch_id, why: g.terminal_local });
      return false;
    }
    try {
      var ph = global.__GR_PACK_HEALTH__ && global.__GR_PACK_HEALTH__[g.batch_id];
      if (ph && (ph.run === "unsupported" || ph.capability === "unsupported")) {
        g.terminal_local = "unsupported";
        stats.unsupported_stops = (stats.unsupported_stops || 0) + 1;
        return false;
      }
    } catch (eU) {}
    try {
      var MM0 = global.GRProbeMethodMatrix;
      if (MM0 && MM0.isUnsupported && MM0.isUnsupported(g.last_err || "")) {
        g.terminal_local = "unsupported";
        return false;
      }
      if (MM0 && MM0.shouldKick && !MM0.shouldKick(g.batch_id, g.collect_attempts, g.last_err, g.last_method)) {
        timelinePush("skip_collect", { batch_id: g.batch_id, why: "method_budget" });
        return false;
      }
    } catch (eM0) {}
    if (g.collect_attempts >= collectMax(g.batch_id)) {
      g.local_phase = "collect_exhausted";
      // Soft/deepen exhaust: trip circuit instead of spinning.
      if (isDeepenBatch(g.batch_id) || isSoftBatch(g.batch_id)) {
        adaptive.pause_deepen_until = now() + (CFG.deepen_circuit_quiet_ms || 20000);
        stats.circuit_trips++;
      }
      return false;
    }
    var t = now();
    if (g.last_collect_ms && t - g.last_collect_ms < CFG.collect_min_gap_ms) return false;
    // Don't thrash if upload already owns the capture.
    if (hasPending(g.batch_id)) return false;
    var st = materialState(g.batch_id);
    if (st === "acked") return false;

    var Col = global.GRCollectors;
    var def = Col && Col.get && Col.get(g.batch_id);
    if (!def || typeof def.run !== "function") {
      // Hard packs may still be loading.
      try {
        if (global.ensureStaticHardModules) global.ensureStaticHardModules();
      } catch (eE) {}
      return false;
    }

    clearKicked(g.batch_id);
    if (g.force_recollect) {
      try {
        if (global.GRUploadQueue && GRUploadQueue.clearSentKeys) {
          GRUploadQueue.clearSentKeys([
            { session_id: sid(), batch_id: g.batch_id, source: g.source || "main" },
          ]);
        }
      } catch (eC) {}
    }

    g.collect_attempts++;
    g.last_collect_ms = t;
    g.run_started_ms = t;
    g.local_phase = "running";
    g.reason = why || "collect";
    g._soft_stall_done = false;
    stats.c_kicks++;
    timelinePush("kick_collect", {
      batch_id: g.batch_id,
      attempt: g.collect_attempts,
      why: why || "collect",
      mode: adaptive.mode,
      method: g.last_method && g.last_method.method_id,
      profile: g.last_method && g.last_method.profile,
    });
    clog("info", "kick_collect " + g.batch_id, {
      attempt: g.collect_attempts,
      why: why || "collect",
      mode: adaptive.mode,
      may_heavy: mayHeavyCollect(),
      method: g.last_method && g.last_method.method_id,
    });

    var packs = [
      {
        id: g.batch_id,
        pack_id: g.batch_id,
        batch_id: g.batch_id,
        priority: g.priority || 1000,
        schedule: "self_heal",
        force_recollect: !!g.force_recollect,
        run: function (c) {
          return def.run(c);
        },
      },
    ];
    // Companion B7 when healing B10 (historical hard-anchor pair).
    if (isB10(g.batch_id)) {
      try {
        var b7 = Col.get && Col.get("B7_sandbox");
        if (b7 && typeof b7.run === "function" && !isSatisfied(ensureGap("B7_sandbox", "main"))) {
          packs.push({
            id: "B7_sandbox",
            pack_id: "B7_sandbox",
            batch_id: "B7_sandbox",
            priority: 900,
            schedule: "self_heal",
            run: function (c) {
              return b7.run(c);
            },
          });
        }
      } catch (e7) {}
    }

    var ctx = {
      session_id: sid(),
      queue: global.GRUploadQueue,
      apiBase:
        (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) ||
        global.__GR_API_BASE__ ||
        "",
      site_id: global.__GR_SITE_ID__ || "",
      source: g.source || "main",
    };

    try {
      // Allow probe despite thin stop flags (gap-fill).
      if (global.__GR_STOP_PROBE__ && !global.__GR_SKIP_IDENTITY__) {
        global.__GR_STOP_PROBE__ = false;
        global.__GR_HALT_UPLOADS__ = false;
        if (global.GRUploadQueue && GRUploadQueue.resume) {
          GRUploadQueue.resume(sid());
        }
      }
    } catch (eR) {}

    ops(
      "gap_c_retry",
      "self_heal",
      {
        batch_id: g.batch_id,
        attempt: g.collect_attempts,
        why: why || "collect",
        force: !!g.force_recollect,
        session_id: sid(),
      },
      g.collect_attempts >= 4 ? "warn" : "info"
    );

    try {
      if (global.GRPackLoader && GRPackLoader.kickAll) {
        GRPackLoader.kickAll(packs, ctx);
        return true;
      }
    } catch (eK) {
      ops("gap_c_kick_fail", "self_heal", { batch_id: g.batch_id, err: String(eK && eK.message || eK) }, "warn");
    }
    return false;
  }

  function loopC(g) {
    if (!g || !g.desired || isSatisfied(g)) return;
    // Prefer transport if material already exists.
    var phase = refreshLocalPhase(g);
    if (phase === "queued" || phase === "uploading" || phase === "retry_wait" || phase === "collected") {
      return;
    }
    if (phase === "running") {
      var elapsed = g.run_started_ms ? now() - g.run_started_ms : 0;
      var soft = softStallMs(g.batch_id);
      var to = runTimeoutMs(g.batch_id);
      // Soft stall: recover early (smart) instead of waiting full hard timeout.
      if (elapsed > soft && !g._soft_stall_done) {
        g._soft_stall_done = true;
        timelinePush("soft_stall", { batch_id: g.batch_id, elapsed: elapsed, soft: soft });
        ops(
          "probe_soft_stall",
          "self_heal",
          { batch_id: g.batch_id, elapsed_ms: elapsed, soft_ms: soft, session_id: sid() },
          "warn"
        );
        if (isB10(g.batch_id) || isHard(g.batch_id)) {
          recoverFromStall(g, "soft_stall");
        }
        return;
      }
      if (g.run_started_ms && elapsed > to) {
        // Hard timeout mid-run — free kicked flag and recollect.
        clearKicked(g.batch_id);
        g.run_started_ms = 0;
        g.local_phase = "collect_timeout";
        g._soft_stall_done = false;
        timelinePush("hard_timeout", { batch_id: g.batch_id, elapsed: elapsed, to: to });
        recoverFromStall(g, "run_timeout");
        ops(
          "gap_c_timeout",
          "self_heal",
          {
            batch_id: g.batch_id,
            timeout_ms: to,
            session_id: sid(),
            gl_hard_lose: !!(global.__GR_LAST_GL_HARD_LOSE__),
          },
          "warn"
        );
      }
      return;
    }
    if (phase === "collect_orphan" || phase === "kicked" || phase === "absent" || phase === "stall_recover") {
      // kicked without pending/ack → must recollect (iss/70 hole).
      if (phase === "kicked" && hasPending(g.batch_id)) return;
      if (!watchdogReady(g) && phase !== "collect_orphan" && g.collect_attempts > 0) return;
      if (phase === "collect_orphan" || phase === "kicked" || phase === "absent" || phase === "stall_recover") {
        kickCollect(g, phase === "absent" ? "missing" : phase);
      }
    }
  }

  // ---------- Loop B: brain reconcile ----------
  function apiBase() {
    try {
      return String(
        (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) ||
          global.__GR_API_BASE__ ||
          ""
      ).replace(/\/$/, "");
    } catch (e) {
      return "";
    }
  }

  function mergeRoutePlan(plan, rev) {
    if (!plan || typeof plan !== "object") return;
    var epoch =
      plan.plan_epoch != null
        ? plan.plan_epoch
        : plan.plan_version != null
          ? plan.plan_version
          : rev;
    if (lastPlanEpoch != null && String(epoch) !== String(lastPlanEpoch)) {
      brainApplyN = 0;
    }
    lastPlanEpoch = epoch;
    try {
      global.__GR_PLAN_EPOCH__ = epoch;
      global.__GR_ROUTE_PLAN__ = plan;
    } catch (e) {}

    var packs = plan.packs || [];
    for (var i = 0; i < packs.length; i++) {
      var p = packs[i] || {};
      var id = p.pack_id || p.id || p.batch_id;
      if (!id) continue;
      var force = p.force_recollect === true;
      ensureGap(id, p.source || "main", {
        desired: true,
        force_recollect: force || undefined,
        priority: p.priority != null ? p.priority : isB10(id) ? 1000 : 200,
        reason: "route_plan",
      });
      if (force) {
        var g = ensureGap(id, p.source || "main");
        g.force_recollect = true;
        g.generation = (g.generation || 1) + 1;
        g.server_seen = false;
      }
    }

    // Always ensure silicon anchors while not cool.
    ensureGap("B10_hw_curves", "main", { desired: true, priority: 1000, reason: "anchor" });

    // Coverage hints.
    try {
      var cov = plan.coverage || {};
      if (cov.has_b10 === true) {
        var gb = ensureGap("B10_hw_curves", "main");
        gb.server_seen = true;
      }
    } catch (eC) {}

    if (plan.stop_probe === true) {
      // Do not cool here — only mark; multi-tick / hardFinal owns cool.
    }
  }

  function applyPlan(plan) {
    if (!plan || pageCool() || pageUnloading()) return;
    if (brainApplyN >= CFG.brain_apply_budget) return;
    if (!global.GRPackLoader || typeof GRPackLoader.applyRoutePlan !== "function") return;
    brainApplyN++;
    stats.b_applies++;
    var ctx = {
      session_id: sid(),
      queue: global.GRUploadQueue,
      apiBase: apiBase(),
      site_id: global.__GR_SITE_ID__ || "",
    };
    try {
      if (global.__GR_STOP_PROBE__ && !global.__GR_SKIP_IDENTITY__) {
        global.__GR_STOP_PROBE__ = false;
        global.__GR_HALT_UPLOADS__ = false;
      }
    } catch (e) {}
    ops(
      "gap_b_apply",
      "self_heal",
      {
        plan_epoch: lastPlanEpoch,
        packs_n: (plan.packs || []).length,
        n: brainApplyN,
        session_id: sid(),
      },
      "info"
    );
    try {
      GRPackLoader.applyRoutePlan(plan, ctx);
    } catch (eA) {}
  }

  function pollAnalyses() {
    var base = apiBase();
    var s = sid();
    if (!base || !s || typeof fetch !== "function") return Promise.resolve(null);
    stats.b_polls++;
    var url = base + "/v1/session/" + encodeURIComponent(s) + "/analyses";
    return fetch(url, {
      method: "GET",
      credentials: "include",
      headers: { accept: "application/json" },
    })
      .then(function (r) {
        if (!r || !r.ok) return null;
        return r.json();
      })
      .then(function (j) {
        if (!j) return null;
        // Support list or single latest.
        var result = j.result || j;
        if (Array.isArray(j.analyses) && j.analyses.length) {
          result = j.analyses[0].result || j.analyses[0];
        }
        if (j.latest) result = j.latest.result || j.latest;
        return result;
      })
      .catch(function () {
        return null;
      });
  }

  function loopB(force) {
    if (pageCool() || pageUnloading()) return Promise.resolve();
    var t = now();
    var hasOpenGaps = false;
    Object.keys(gaps).forEach(function (k) {
      if (gaps[k].desired && !isSatisfied(gaps[k])) hasOpenGaps = true;
    });
    var spacing = !visible()
      ? CFG.brain_poll_hidden_ms
      : hasOpenGaps
        ? CFG.brain_poll_gap_ms
        : CFG.brain_poll_visible_ms;
    if (!force && lastBrainPollMs && t - lastBrainPollMs < spacing) return Promise.resolve();
    lastBrainPollMs = t;

    return pollAnalyses().then(function (result) {
      if (!result) {
        // Still run C/T on local desired anchors.
        return;
      }
      try {
        global.__GR_LAST_ANALYZE__ = { result: result, route_plan: result.route_plan };
      } catch (e) {}
      var rev = result.analysis_rev || result.rev || 0;
      if (rev) lastBrainRev = Math.max(lastBrainRev, rev);

      // Server terminal → stop self-heal collect (uploads may still flush).
      try {
        var cps = result.cycle_probe_status || {};
        var cov = result.coverage || cps.identity_coverage || {};
        if (
          result.analysis_terminal === true ||
          cps.analysis_terminal === true ||
          ((result.route_plan && result.route_plan.stop_probe) && cov.coverage_complete) ||
          cov.brain_schedule_final === true ||
          cps.brain_schedule_final === true
        ) {
          if (cov.has_b10 === true || cps.has_b10 === true || materialState("B10_hw_curves") === "acked") {
            // Mark B10 satisfied.
            var gDone = ensureGap("B10_hw_curves", "main");
            gDone.server_seen = true;
          }
        }
        if (cov.has_b10 === true || cps.has_b10 === true) {
          ensureGap("B10_hw_curves", "main", { server_seen: true });
        }
        var recIds = cov.received_batch_ids || cps.received_batch_ids;
        if (Array.isArray(recIds)) {
          recIds.forEach(function (id) {
            ensureGap(id, "main", { server_seen: true });
          });
        }
      } catch (eT) {}

      var plan =
        result.route_plan || (result.brain && result.brain.route_plan) || global.__GR_ROUTE_PLAN__;
      if (plan) {
        mergeRoutePlan(plan, rev);
        // Apply when there are unsatisfied desired packs from plan.
        var needApply = false;
        (plan.packs || []).forEach(function (p) {
          var id = p.pack_id || p.id;
          if (!id) return;
          var g = ensureGap(id, p.source || "main");
          if (!isSatisfied(g)) needApply = true;
          if (p.force_recollect) needApply = true;
        });
        if (needApply) applyPlan(plan);
      }
    });
  }

  // ---------- Supervisor ----------
  function kickBootIfNoSession() {
    if (sid()) return;
    if (pageUnloading()) return;
    try {
      var tNow = now();
      if (stats._last_boot_kick_ms && tNow - stats._last_boot_kick_ms < 4000) return;
      stats._last_boot_kick_ms = tNow;
      stats.boot_kicks = (stats.boot_kicks || 0) + 1;
      timelinePush("boot_kick", { why: "no_session" });
      var Boot = global.GRBoot || global.Boot;
      if (Boot && typeof Boot.start === "function") {
        Boot.start({ force_identity: !!global.__GR_FORCE_IDENTITY__ });
      }
    } catch (eK) {}
  }

  function tickOnce(forceBrain) {
    if (!started || pageUnloading()) return;
    stats.ticks++;
    applyAdaptivePolicy(false);
    kickBootIfNoSession();
    // Seed anchors while probing.
    if (!pageCool()) {
      ensureGap("B10_hw_curves", "main", { desired: true, priority: 1000, reason: "anchor" });
      ensureGap("B0_bootstrap", "main", { desired: true, priority: 900, reason: "anchor" });
    }

    var keys = Object.keys(gaps);
    // Priority: hard/B10 first.
    keys.sort(function (a, b) {
      return (gaps[b].priority || 0) - (gaps[a].priority || 0);
    });

    var b10Ok = false;
    try {
      var gB10 = ensureGap("B10_hw_curves", "main");
      b10Ok = !!(gB10 && isSatisfied(gB10));
    } catch (eB) {}

    for (var i = 0; i < keys.length; i++) {
      var g = gaps[keys[i]];
      if (!g.desired) continue;
      if (isSatisfied(g)) {
        if (g.local_phase !== "acked") {
          g.local_phase = "acked";
          stats.satisfied++;
          ops(
            "gap_satisfied",
            "self_heal",
            { batch_id: g.batch_id, session_id: sid() },
            "info"
          );
          timelinePush("satisfied", { batch_id: g.batch_id });
          // Primary B10 landed — clear deepen pause if any.
          if (isB10(g.batch_id) && adaptive.pause_deepen_until) {
            adaptive.pause_deepen_until = 0;
            applyAdaptivePolicy(true);
          }
        }
        continue;
      }
      if (pageCool()) {
        // Cool: transport-only flush for anything already collected.
        loopT(g);
        continue;
      }
      // Recovery mode: only hard anchors until B10 ok.
      if (adaptive.mode === "recovery" && !b10Ok && !isB10(g.batch_id) && !isHard(g.batch_id)) {
        continue;
      }
      refreshLocalPhase(g);
      loopT(g);
      loopC(g);
    }

    // Brain reconcile (async) — skip deepen plan spam while recovering.
    if (!(adaptive.mode === "recovery" && !b10Ok)) {
      loopB(!!forceBrain);
    }

    // Progressive land while window open: keep pumping uploads (do not wait for pagehide).
    // Re-probe after product VERSION bump lands new opaque packs via boot+scheduler;
    // this loop only ensures already-collected / pending batches reach the server.
    try {
      var Qp = global.GRUploadQueue;
      if (Qp && !pageUnloading() && !pageCool()) {
        if (typeof Qp.pumpWhileOpen === "function") {
          Qp.pumpWhileOpen("self_heal_progressive");
        } else if (typeof Qp.flush === "function") {
          Qp.flush("self_heal_progressive");
        }
        // Emit completeness ops every ~10s while hard anchors still missing
        var snap =
          typeof Qp.completenessSnapshot === "function" ? Qp.completenessSnapshot() : null;
        if (snap && !snap.main_complete) {
          var tNow = now();
          if (!stats._last_completeness_ms || tNow - stats._last_completeness_ms >= 10000) {
            stats._last_completeness_ms = tNow;
            ops(
              "probe_completeness_progressive",
              "self_heal",
              {
                session_id: sid(),
                product_version: snap.product_version,
                probe_depth_class: snap.probe_depth_class,
                hard_ok_n: snap.hard_ok_n,
                hard_total: snap.hard_total,
                hard_missing: (snap.hard_missing || []).slice(0, 12),
                batches_ok_n: snap.batches_ok_n,
                pending: snap.pending,
                inflight: snap.inflight,
                progressive: true,
              },
              "info"
            );
          }
        }
      }
    } catch (eProg) {}
  }

  // Completion-driven reconcile: upload/collect events should refill free
  // lanes promptly instead of waiting for the 2.5–4s periodic tick. Coalesce
  // bursts from a parallel wave into one brain request.
  function scheduleEventTick(reason) {
    if (!started || pageUnloading() || eventTickTimer) return;
    eventTickTimer = setTimeout(function () {
      eventTickTimer = null;
      try {
        tickOnce(reason === "upload_ok");
      } catch (eTick) {}
    }, 150);
  }

  function scheduleNext() {
    if (!started) return;
    if (timer) {
      try {
        clearTimeout(timer);
      } catch (e) {}
      timer = null;
    }
    var ms = visible() ? CFG.tick_visible_ms : CFG.tick_hidden_ms;
    timer = setTimeout(function () {
      try {
        tickOnce();
      } catch (eT) {}
      scheduleNext();
    }, ms);
  }

  function start(opts) {
    opts = opts || {};
    if (opts.cfg && typeof opts.cfg === "object") {
      Object.keys(opts.cfg).forEach(function (k) {
        if (CFG[k] != null) CFG[k] = opts.cfg[k];
      });
    }
    applyAdaptivePolicy(true);
    if (!eventListenersBound) {
      try {
        ["gr-upload-ok", "gr-collect-fail", "gr-upload-terminal"].forEach(function (name) {
          global.addEventListener(name, function () {
            scheduleEventTick(name === "gr-upload-ok" ? "upload_ok" : "collect_event");
          });
        });
        eventListenersBound = true;
      } catch (eEvents) {}
    }
    try {
      if (global.__GR_FORCE_IDENTITY__ || global.__GR_FORCE_REPROBE__) {
        adaptive.stall_recoveries = Math.max(adaptive.stall_recoveries || 0, 2);
        applyAdaptivePolicy(true);
      }
    } catch (eFr) {}
    // Origin coordinator: multi-tab heavy election (same origin only).
    try {
      if (global.GROriginCoordinator && GROriginCoordinator.start) {
        GROriginCoordinator.start({
          cycle_id: sid(),
          why: "self_heal_start",
        });
      }
    } catch (eGpi) {}
    // Seed session hard anchors.
    ensureGap("B10_hw_curves", "main", { desired: true, priority: 1000, reason: "start" });
    ensureGap("B0_bootstrap", "main", { desired: true, priority: 900, reason: "start" });
    if (opts.missing && Array.isArray(opts.missing)) {
      opts.missing.forEach(function (id) {
        ensureGap(id, "main", { desired: true, reason: "open_missing" });
      });
    }
    if (opts.need_hard_anchor) {
      ensureGap("B10_hw_curves", "main", {
        desired: true,
        priority: 1000,
        reason: "need_hard_anchor",
      });
    }
    started = true;
    try {
      global.__GR_SELF_HEAL_ACTIVE__ = true;
    } catch (e) {}
    timelinePush("supervisor_start", {
      engine: adaptive.engine,
      mode: adaptive.mode,
      gpi_role:
        (global.GROriginCoordinator &&
          GROriginCoordinator.getRole &&
          GROriginCoordinator.getRole()) ||
        null,
    });
    ops(
      "gap_supervisor_start",
      "self_heal",
      {
        session_id: sid(),
        engine: adaptive.engine,
        mode: adaptive.mode,
        max_light: adaptive.max_light,
        gpi_role:
          (global.GROriginCoordinator &&
            GROriginCoordinator.getRole &&
            GROriginCoordinator.getRole()) ||
          null,
      },
      "info"
    );
    // Immediate pass + schedule.
    try {
      tickOnce();
    } catch (e0) {}
    scheduleNext();
    return api;
  }

  function stop(why) {
    started = false;
    if (timer) {
      try {
        clearTimeout(timer);
      } catch (e) {}
      timer = null;
    }
    try {
      global.__GR_SELF_HEAL_ACTIVE__ = false;
    } catch (e2) {}
    ops("gap_supervisor_stop", "self_heal", { why: why || "stop", session_id: sid() }, "info");
  }

  function seedFromOpen(opened) {
    opened = opened || {};
    var missing = opened.missing_batches || opened.missing || [];
    if (Array.isArray(missing)) {
      missing.forEach(function (id) {
        ensureGap(id, "main", { desired: true, reason: "open_missing" });
      });
    }
    if (opened.need_hard_anchor || opened.resumed) {
      ensureGap("B10_hw_curves", "main", {
        desired: true,
        priority: 1000,
        reason: opened.need_hard_anchor ? "need_hard_anchor" : "resumed",
      });
    }
    // last identity coverage
    try {
      var lir = opened.last_identity_result || {};
      var plan = lir.route_plan || (lir.brain && lir.brain.route_plan);
      if (plan) mergeRoutePlan(plan, lir.analysis_rev || 0);
      var cov = lir.coverage || {};
      if (cov.has_b10) ensureGap("B10_hw_curves", "main", { server_seen: true });
    } catch (e) {}
    if (!started) start({ need_hard_anchor: !!opened.need_hard_anchor });
    else {
      try {
        tickOnce();
      } catch (eT) {}
    }
  }

  function onVisible() {
    if (!started || pageCool()) return;
    lastBrainPollMs = 0;
    try {
      tickOnce();
      loopB(true);
    } catch (e) {}
  }

  function snapshot() {
    var out = [];
    Object.keys(gaps).forEach(function (k) {
      var g = gaps[k];
      out.push({
        batch_id: g.batch_id,
        source: g.source,
        desired: g.desired,
        server_seen: g.server_seen,
        phase: g.local_phase,
        collect_attempts: g.collect_attempts,
        transport_nudges: g.transport_nudges,
        force_recollect: g.force_recollect,
        reason: g.reason,
        satisfied: isSatisfied(g),
        run_elapsed_ms:
          g.run_started_ms && g.local_phase === "running" ? now() - g.run_started_ms : 0,
      });
    });
    var gpi = null;
    try {
      if (global.GROriginCoordinator && GROriginCoordinator.snapshot) {
        gpi = GROriginCoordinator.snapshot();
      }
    } catch (eG) {}
    return {
      started: started,
      session_id: sid(),
      stats: Object.assign({}, stats),
      gaps: out,
      cfg: Object.assign({}, CFG),
      adaptive: Object.assign({}, adaptive),
      timeline_tail: timeline.slice(-24),
      engine: adaptive.engine,
      mode: adaptive.mode,
      gpi: gpi,
      may_heavy: mayHeavyCollect(),
    };
  }

  // Visibility / online hooks (idempotent).
  try {
    if (!global.__GR_SELF_HEAL_HOOKS__) {
      global.__GR_SELF_HEAL_HOOKS__ = true;
      if (typeof document !== "undefined" && document.addEventListener) {
        document.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "visible") onVisible();
        });
      }
      global.addEventListener("online", function () {
        onVisible();
      });
      global.addEventListener("pageshow", function () {
        onVisible();
      });
    }
  } catch (eH) {}

  var api = {
    __ready: true,
    CFG: CFG,
    start: start,
    stop: stop,
    seedFromOpen: seedFromOpen,
    applyAdaptivePolicy: applyAdaptivePolicy,
    recoverFromStall: recoverFromStall,
    detectEngine: detectEngine,
    timeline: function () {
      return timeline.slice();
    },
    ensureGap: ensureGap,
    mergeRoutePlan: mergeRoutePlan,
    tick: tickOnce,
    loopB: loopB,
    onVisible: onVisible,
    snapshot: snapshot,
    isSatisfied: function (bid) {
      var g = gaps[gapKey(bid, "main")];
      return g ? isSatisfied(g) : serverHasBatch(bid) || materialState(bid) === "acked";
    },
  };

  global.GRProbeSelfHeal = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- ops_report.js ---- */
/**
 * Silent client ops reporter — never touches console for its own traffic.
 * POSTs compact health/error/console events to first-party /v1/ops/client_event.
 * Always includes product_version for version-scoped error triage.
 */
(function (global) {
  "use strict";
  var ENDPOINT = "/v1/ops/client_event";
  // Open collection: low site traffic — allow richer multipath/upload diagnostics.
  var MAX_PER_MIN = 80;
  var sent = [];
  var halted = false;
  var seenConsoleKeys = Object.create(null);
  var CONSOLE_DEDUP_MS = 30000;

  /**
   * Client error/ops upload enable (default on).
   * Disable via: __GR_BOOT__.ops_client_events=false | __GR_OPS_CLIENT_EVENTS__=0 | GROps.setEnabled(false)
   * Server may also reject when admin setting ops_client_events_enabled=0.
   */
  function clientEventsEnabled() {
    try {
      if (global.__GR_OPS_CLIENT_EVENTS__ === 0 || global.__GR_OPS_CLIENT_EVENTS__ === false)
        return false;
      if (global.__GR_OPS_CLIENT_EVENTS__ === 1 || global.__GR_OPS_CLIENT_EVENTS__ === true)
        return true;
      var boot = global.__GR_BOOT__ || {};
      if (boot.ops_client_events === false || boot.ops_client_events === 0) return false;
      if (boot.opsClientEvents === false || boot.opsClientEvents === 0) return false;
      if (String(boot.ops_client_events || "").toLowerCase() === "off") return false;
    } catch (e) {}
    return !halted;
  }

  function apiBase() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var b = String(boot.apiBase || boot.first_party_path || global.__GR_API_BASE__ || "/g5");
      return b.replace(/\/$/, "") || "/g5";
    } catch (e) {
      return "/g5";
    }
  }

  function prune() {
    var now = Date.now();
    sent = sent.filter(function (t) {
      return now - t < 60000;
    });
  }

  function uaHash() {
    try {
      var ua = String((global.navigator && navigator.userAgent) || "");
      var h = 2166136261;
      for (var i = 0; i < ua.length; i++) {
        h ^= ua.charCodeAt(i);
        h = Math.imul(h, 16777619);
      }
      return ("00000000" + (h >>> 0).toString(16)).slice(-8);
    } catch (e) {
      return "";
    }
  }

  /**
   * Engine family without InstallTrigger (deprecated — typeof alone warns in Firefox).
   * Capability-first: mozInnerScreenX / -moz- CSS; then real Gecko UA (not "like Gecko").
   */
  function isGeckoEngine(ua) {
    ua = String(ua || "");
    try {
      if (typeof global.mozInnerScreenX === "number") return true;
    } catch (e0) {}
    try {
      if (
        typeof CSS !== "undefined" &&
        CSS.supports &&
        CSS.supports("-moz-appearance", "none") &&
        (/Firefox\//.test(ua) || /FxiOS\//.test(ua) || (/Gecko\//.test(ua) && !/like Gecko/.test(ua)))
      ) {
        return true;
      }
    } catch (e1) {}
    if (/Firefox\//.test(ua) || /FxiOS\//.test(ua)) return true;
    if (/Gecko\//.test(ua) && !/like Gecko/.test(ua) && !/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua))
      return true;
    return false;
  }

  function engineFamily() {
    try {
      var ua = String((global.navigator && navigator.userAgent) || "");
      if (isGeckoEngine(ua)) return "gecko";
      if (/AppleWebKit\/605/.test(ua) && /Safari\//.test(ua) && !/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua))
        return "webkit";
      if (/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua) || /CriOS\//.test(ua)) return "blink";
      if (/AppleWebKit\//.test(ua) && /Safari\//.test(ua) && !/Chrome\/|Chromium\/|CriOS\//.test(ua))
        return "webkit";
      return "unknown";
    } catch (e2) {
      return "unknown";
    }
  }

  function productVersion() {
    try {
      var boot = global.__GR_BOOT__ || {};
      return String(
        boot.version ||
          boot.product_version ||
          global.__GR_PRODUCT_VERSION__ ||
          ""
      );
    } catch (e) {
      return "";
    }
  }

  /**
   * Resolve visitor_terminal_id from all known FE surfaces so upload error/warn
   * rows always join to VT (retry outcome verification).
   */
  function resolveVisitorTerminalId() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var candidates = [
        global.__GR_VTID__,
        boot.visitor_terminal_id,
        boot.visitorTerminalId,
        boot.vt,
        global.__GR_VISITOR_TERMINAL_ID__,
      ];
      try {
        if (global.GRStorage && typeof GRStorage.getVt === "function") {
          candidates.push(GRStorage.getVt());
        }
      } catch (eS) {}
      try {
        if (typeof localStorage !== "undefined") {
          candidates.push(localStorage.getItem("gr_visitor_terminal_v1"));
          candidates.push(localStorage.getItem("_g5_vt"));
        }
      } catch (eL) {}
      for (var i = 0; i < candidates.length; i++) {
        var v = candidates[i];
        if (v != null && String(v).trim()) {
          var s = String(v).trim().slice(0, 96);
          try {
            global.__GR_VTID__ = s;
          } catch (eSet) {}
          return s;
        }
      }
    } catch (eR) {}
    return "";
  }

  function resolveSessionId() {
    try {
      return String(
        global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.session_id) ||
          ""
      ).slice(0, 96);
    } catch (e) {
      return "";
    }
  }

  /**
   * High-volume lifecycle/noise codes — sample to keep ops usable.
   * Real faults (upload_5xx, hard_load_fail, open_fail, version_heal) stay 1.0.
   */
  var SAMPLE_RATES = {
    // Lifecycle remint is expected on navigation; keep tiny sample (info severity).
    cycle_remint: 0.02,
    vt_mint: 0.04,
    fe_diag: 0.02,
    hard_sla_retry: 0.1,
    need_hard_anchor: 0.1,
    privacy_guard: 0.02,
    multipath_done: 0.05,
    // Network fails are environmental (CF challenge / tab kill) — sample harder.
    upload_network: 0.2,
    upload_upstream_closed: 0.25,
    rpa_quiet: 0.1,
    upload_soft_exhausted: 0.2,
    upload_deepen_exhausted: 0.25,
    upload_hard_exhausted: 0.35,
    fe_pack_reload: 0.03,
    wave2_defer_b10: 0.08,
    b10_content_gate: 0.15,
    upload_4xx: 0.15,
    wave2_empty: 0.12,
    // Self-heal Gap Table (T/C/B) — sample; start/stop always useful once.
    gap_t_retry: 0.2,
    gap_c_retry: 0.35,
    gap_c_timeout: 0.6,
    gap_b_apply: 0.2,
    gap_satisfied: 0.15,
    gap_supervisor_start: 1,
    gap_supervisor_stop: 1,
    cool_without_silicon: 0.2,
    // Seal/WASM client failures are not HTTP 5xx — sample + hard cap (crawler storms).
    upload_seal_fail: 0.35,
    seal_wasm_fail: 0.25,
    seal_wasm_unsupported: 1,
    // Recovery / outcome always kept — verify retry eventually landed.
    upload_recovered: 1,
    upload_session_summary: 1,
    b10x_must_land_scheduled: 0.15,
  };
  var sampleSeen = Object.create(null);
  var codeCaps = Object.create(null);
  var CODE_CAPS = {
    cycle_remint: 1,
    vt_mint: 2,
    upload_4xx: 6,
    rpa_quiet: 3,
    upload_soft_exhausted: 4,
    upload_deepen_exhausted: 4,
    upload_hard_exhausted: 4,
    fe_pack_reload: 2,
    wave2_defer_b10: 2,
    b10_content_gate: 3,
    gap_t_retry: 8,
    gap_c_retry: 10,
    gap_c_timeout: 6,
    gap_b_apply: 8,
    gap_satisfied: 6,
    gap_supervisor_start: 2,
    gap_supervisor_stop: 2,
    fe_console_error: 5,
    upload_network: 8,
    upload_upstream_closed: 8,
    wave2_empty: 2,
    upload_recovered: 12,
    // Crawlers/fake kernels retry seal dozens of times — keep first few only.
    upload_seal_fail: 4,
    seal_wasm_fail: 3,
    seal_wasm_unsupported: 2,
    upload_5xx: 12,
  };

  function shouldSample(code) {
    var c = String(code || "");
    var cap = CODE_CAPS[c];
    if (cap != null) {
      codeCaps[c] = (codeCaps[c] || 0) + 1;
      if (codeCaps[c] > cap) return false;
    }
    var rate = SAMPLE_RATES[c];
    if (rate == null || rate >= 1) return true;
    // Always keep first event per code per page for triage, then sample.
    if (!sampleSeen[c]) {
      sampleSeen[c] = 1;
      return true;
    }
    try {
      return Math.random() < rate;
    } catch (eS) {
      return true;
    }
  }

  function report(code, stage, detail, severity) {
    if (!clientEventsEnabled() || !code) return;
    // Drop bogus empty fe_diag spam (was console.warn replaced with report("fe_diag")).
    if (String(code) === "fe_diag") {
      var d0 = detail && typeof detail === "object" ? detail : {};
      if (!d0.msg && !d0.err && !d0.detail && stage === "boot") return;
    }
    if (!shouldSample(code)) return;
    prune();
    if (sent.length >= MAX_PER_MIN) return;
    sent.push(Date.now());
    var boot = global.__GR_BOOT__ || {};
    var rate = SAMPLE_RATES[String(code)] != null ? SAMPLE_RATES[String(code)] : 1;
    var vt = resolveVisitorTerminalId();
    var sid = resolveSessionId();
    var body = {
      code: String(code).slice(0, 64),
      stage: String(stage || "client").slice(0, 32),
      severity: severity || "error",
      ts_ms: Date.now(),
      site_id: boot.site_id || boot.siteId || global.__GR_SITE_ID__ || "",
      visitor_terminal_id: vt,
      session_id: sid,
      product_version: productVersion(),
      inject_path: boot.injectPath || boot.inject_path || "",
      engine_family: engineFamily(),
      ua_hash: uaHash(),
      detail: detail && typeof detail === "object" ? detail : {},
      sample_rate: rate,
    };
    // Always mirror version + vtid/session inside detail for join/triage.
    try {
      if (body.detail && typeof body.detail === "object") {
        if (!body.detail.product_version) body.detail.product_version = body.product_version;
        if (!body.detail.visitor_terminal_id && vt) body.detail.visitor_terminal_id = vt;
        if (!body.detail.session_id && sid) body.detail.session_id = sid;
      }
    } catch (eD) {}
    var url = apiBase() + ENDPOINT;
    try {
      var blob = new Blob([JSON.stringify(body)], { type: "text/plain" });
      if (global.navigator && typeof navigator.sendBeacon === "function") {
        if (navigator.sendBeacon(url, blob)) return;
      }
    } catch (eB) {}
    try {
      if (typeof fetch === "function") {
        fetch(url, {
          method: "POST",
          credentials: "same-origin",
          keepalive: true,
          headers: { "Content-Type": "text/plain" },
          body: JSON.stringify(body),
        }).catch(function () {});
      }
    } catch (eF) {}
  }

  /** Gated debug log — only when server/open enabled debug. Never default-on. */
  function dlog() {
    try {
      if (!(global.__GR_DEBUG__ || (global.__GR_BOOT__ && global.__GR_BOOT__.debug))) return;
      if (arguments && arguments.length) {
        report("debug_trace", "debug", { n: arguments.length }, "info");
      }
    } catch (e) {}
  }

  /** Browser / extension noise we must not flood into ops. */
  function isBrowserNoise(msg) {
    msg = String(msg || "");
    if (!msg) return true;
    return /InstallTrigger is deprecated|Fingerprinting Protection is altering|WebGPU is experimental|Failed to create WebGPU Context Provider|gpuweb\/wiki\/Implementation|moz-extension:|chrome-extension:|Download the React DevTools|\[HMR\]|webpack|favicon\.ico|net::ERR_BLOCKED_BY_CLIENT|ResizeObserver loop|Script error\.$|Script terminated by timeout|\[NEW\] Explain Console|无法访问Iframe|cross-origin frame|Blocked a frame with origin|Falling back to browser navigation|Failed to fetch RSC payload|SecurityError.*frame|from accessing a cross-origin/i.test(
      msg
    );
  }

  function argsToMsg(args) {
    try {
      var parts = [];
      for (var i = 0; i < (args ? args.length : 0) && i < 6; i++) {
        var a = args[i];
        if (a == null) {
          parts.push(String(a));
        } else if (typeof a === "string") {
          parts.push(a);
        } else if (a && a.message) {
          parts.push(String(a.message));
        } else if (a && a.stack) {
          parts.push(String(a.stack).slice(0, 200));
        } else {
          try {
            parts.push(JSON.stringify(a).slice(0, 160));
          } catch (eJ) {
            parts.push(String(a));
          }
        }
      }
      return parts.join(" ").slice(0, 360);
    } catch (e) {
      return "";
    }
  }

  function consoleDedupOk(key) {
    var now = Date.now();
    var prev = seenConsoleKeys[key] || 0;
    if (now - prev < CONSOLE_DEDUP_MS) return false;
    seenConsoleKeys[key] = now;
    // Bound map size
    var keys = Object.keys(seenConsoleKeys);
    if (keys.length > 80) {
      for (var i = 0; i < 40; i++) delete seenConsoleKeys[keys[i]];
    }
    return true;
  }

  function reportConsole(level, args) {
    try {
      var msg = argsToMsg(args);
      if (!msg || isBrowserNoise(msg)) return;
      // Prefer GR / probe-relevant lines; still take bare Error / TypeError from our pages.
      var relevant =
        /gr|GR|WebGL|WEBGL|probe|registry|upload|ingest|B\d+_|\bR\d+_spot|pack_loader|context lost|INVALID_ENUM|out of memory|TypeError|ReferenceError|SyntaxError|Failed to fetch|NetworkError/i.test(
          msg
        );
      if (!relevant && level === "warn") return;
      if (!relevant && level === "error") {
        // Still capture generic errors when on our SDK path (filename-less console.error).
        if (!/error|fail|exception|reject/i.test(msg)) return;
      }
      var code = level === "warn" ? "fe_console_warn" : "fe_console_error";
      var sev = level === "warn" ? "warn" : "error";
      var dkey = code + ":" + msg.slice(0, 80);
      if (!consoleDedupOk(dkey)) return;
      report(
        code,
        "console",
        {
          level: level,
          msg: msg.slice(0, 320),
          href: (function () {
            try {
              return String((global.location && location.pathname) || "").slice(0, 120);
            } catch (eH) {
              return "";
            }
          })(),
        },
        sev
      );
    } catch (eC) {}
  }

  // Hook runtime errors + console.warn/error (console intercept is the only way
  // to surface WebGL/browser warn lines that never fire window.onerror).
  try {
    if (!global.__GR_OPS_HOOKED__) {
      global.__GR_OPS_HOOKED__ = true;
      var lastGlLost = 0;
      global.addEventListener(
        "webglcontextlost",
        function (ev) {
          var now = Date.now();
          if (now - lastGlLost < 5000) return;
          lastGlLost = now;
          report(
            "webgl_context_lost",
            "webgl",
            {
              status: "lost",
              target: ev && ev.target ? String(ev.target.tagName || "canvas") : "canvas",
            },
            "warn"
          );
        },
        true
      );
      global.addEventListener("error", function (ev) {
        try {
          var msg = String((ev && (ev.message || (ev.error && ev.error.message))) || "");
          if (!msg || isBrowserNoise(msg)) return;
          var src = String((ev && ev.filename) || "");
          var ours =
            /WebGL|WEBGL|Script error|out of memory|Allocation failed|TypeError|ReferenceError/i.test(
              msg
            ) ||
            (src && /gr|registry|collectors|pack_loader|upload_queue|ops_report/i.test(src));
          if (!ours) return;
          report(
            "fe_runtime_error",
            "error",
            {
              msg: msg.slice(0, 160),
              src: src.slice(0, 80),
              line: ev && ev.lineno,
              col: ev && ev.colno,
            },
            "error"
          );
        } catch (eE) {}
      });
      global.addEventListener("unhandledrejection", function (ev) {
        try {
          var reason = ev && ev.reason;
          var msg = String((reason && (reason.message || reason)) || reason || "");
          if (!msg || isBrowserNoise(msg)) return;
          if (/WebGL|upload|probe|GR|multipath|fetch|network|TypeError|reject/i.test(msg)) {
            report(
              "fe_unhandled_rejection",
              "error",
              { msg: msg.slice(0, 160) },
              "warn"
            );
          }
        } catch (eR) {}
      });

      // Wrap console.error / console.warn (preserve original; never throw).
      try {
        var c = global.console;
        if (c && !global.__GR_OPS_CONSOLE__) {
          global.__GR_OPS_CONSOLE__ = true;
          var origError = typeof c.error === "function" ? c.error.bind(c) : null;
          var origWarn = typeof c.warn === "function" ? c.warn.bind(c) : null;
          if (origError) {
            c.error = function () {
              try {
                reportConsole("error", arguments);
              } catch (e1) {}
              try {
                return origError.apply(c, arguments);
              } catch (e2) {}
            };
          }
          if (origWarn) {
            c.warn = function () {
              try {
                reportConsole("warn", arguments);
              } catch (e3) {}
              try {
                return origWarn.apply(c, arguments);
              } catch (e4) {}
            };
          }
        }
      } catch (eHookC) {}
    }
  } catch (eHook) {}

  global.GROps = {
    report: report,
    dlog: dlog,
    engineFamily: engineFamily,
    productVersion: productVersion,
    wave2Empty: function (extra) {
      // Dedupe: hard module race can fire 3× wave2Empty per page.
      try {
        var t = Date.now();
        if (global.__GR_LAST_WAVE2_EMPTY_MS__ && t - global.__GR_LAST_WAVE2_EMPTY_MS__ < 8000) {
          return;
        }
        global.__GR_LAST_WAVE2_EMPTY_MS__ = t;
      } catch (eW) {}
      // Expected pack-schedule lifecycle (hard not ready yet / benign empty).
      report("wave2_empty", "kick", extra || {}, "info");
    },
    hardLoadFail: function (pack, err) {
      report(
        "hard_load_fail",
        "load_pack",
        { pack: String(pack || ""), err: String(err || "").slice(0, 240) },
        "error"
      );
    },
    uploadHttp: function (status, batch, extra) {
      var s = Number(status) || 0;
      extra = extra || {};
      var bid = String(batch || "").slice(0, 48);
      var errStr = String(extra.err || extra.message || extra.body_snip || "");
      var cls = String(extra.net_class || errStr || "").toLowerCase();
      // Never label HTTP 2xx as 4xx (webkit false positive was http:200 + upload_4xx).
      if (s > 0 && s < 400) {
        report(
          "upload_biz_reject",
          "upload",
          Object.assign({ http: s, batch: bid }, extra),
          "warn"
        );
        return;
      }
      // --- Seal / WASM client failures (NOT HTTP 5xx) ---
      // Prod v148: ~half of "upload_5xx" were seal_wasm_required / seal_failed with http:0
      // on crawler/fake-kernel sessions (gateway_only). Classify for ops integrity.
      var sealFlag =
        extra.seal_failed === true ||
        extra.seal_required === true ||
        extra.seal_failed === 1 ||
        String(extra.code || "") === "seal_failed" ||
        String(extra.code || "") === "sealed_required";
      var sealMsg =
        /seal_wasm|seal_failed|seal_module|seal_grant|seal_timeout|wasm_|webassembly/i.test(
          errStr
        ) || /seal_wasm|seal_failed|wasm_/i.test(cls);
      if (sealFlag || sealMsg) {
        var unsup = /wasm_unsupported|webassembly is not defined|no webassembly/i.test(
          errStr + " " + cls
        );
        var wasmLoad =
          unsup ||
          /seal_wasm_required|seal_wasm_load|wasm_http|wasm_not_binary|wasm_no_exports|instantiateStreaming|reached end while decod/i.test(
            errStr
          );
        var codeSeal = unsup
          ? "seal_wasm_unsupported"
          : wasmLoad
            ? "seal_wasm_fail"
            : "upload_seal_fail";
        // Crawler/incomplete engines often retry every batch — warn not error flood.
        var sevSeal = unsup || extra.final === true ? "error" : "warn";
        extra.net_class = extra.net_class || (wasmLoad ? "seal_wasm" : "seal_client");
        extra.seal_failed = true;
        report(
          codeSeal,
          "upload",
          Object.assign(
            {
              http: s,
              batch: bid,
              visitor_terminal_id: resolveVisitorTerminalId(),
              session_id: resolveSessionId(),
            },
            extra
          ),
          sevSeal
        );
        return;
      }
      var code = s >= 500 || s === 0 ? "upload_5xx" : "upload_4xx";
      var sev = "error";
      // Cloudflare / edge challenge (I'm Under Attack, JS challenge, 403 HTML).
      var isCf =
        s === 403 ||
        s === 503 ||
        /challenge|cf-mitigated|access denied|just a moment|attention required|under attack|cf-ray|cloudflare/i.test(
          cls
        );
      if (isCf) {
        code = s === 0 ? "upload_network" : "upload_4xx";
        sev = "warn";
        extra.net_class = extra.net_class || "edge_challenge";
        extra.cf_challenge = true;
      } else if (s === 0 && extra.network) {
        // http:0 + network = browser never got HTTP status (not API 4xx/5xx).
        // Subtypes: offline | client_fetch_fail | edge_challenge | abort | backgrounded.
        code = "upload_network";
        if (extra.online === false || cls === "offline") {
          extra.net_class = "offline";
        } else if (!extra.net_class || extra.net_class === "fetch_fail") {
          extra.net_class = extra.net_class || "client_fetch_fail";
        }
        extra.transport = extra.transport || "no_http_response";
        // Mid-retry network blips → warn; final deepen exhaust also warn (not API outage).
        var willRetry = extra.will_retry !== false && extra.final !== true;
        if (
          extra.backgrounded ||
          extra.timeout_abort ||
          willRetry ||
          /abort|offline|client_fetch/i.test(cls + " " + (extra.net_class || ""))
        ) {
          sev = "warn";
        } else {
          // Final network fail without HTTP → still warn (domain/CDN/client), not 5xx error.
          sev = "warn";
        }
      } else if (s === 0 && !extra.network) {
        // Ambiguous http:0 without network flag — prefer network/warn over false 5xx.
        code = "upload_network";
        sev = "warn";
        extra.net_class = extra.net_class || "http0_unclassified";
        extra.transport = extra.transport || "no_http_response";
      } else if (
        // Edge/proxy dropped the TCP stream mid-response: often surfaces as
        // HTTP 500 + "connection closed" (nginx/CF/browser), not app 5xx logic.
        // Prod v148: large share of human thin_surface upload_5xx was this pattern.
        /connection closed|broken pipe|econnreset|err_connection|connection reset|upstream prematurely|socket hang up/i.test(
          errStr + " " + cls
        )
      ) {
        code = "upload_upstream_closed";
        sev = extra.final === true ? "error" : "warn";
        extra.net_class = extra.net_class || "upstream_closed";
        extra.network = true;
      } else if (s >= 500) {
        code = "upload_5xx";
        sev = "error";
      }
      report(
        code,
        "upload",
        Object.assign(
          {
            http: s,
            batch: bid,
            visitor_terminal_id: resolveVisitorTerminalId(),
            session_id: resolveSessionId(),
          },
          extra
        ),
        sev
      );
    },
    /**
     * Report that a batch eventually uploaded after prior network/HTTP failure.
     * Always includes visitor_terminal_id for retry-outcome verification.
     */
    uploadRecovered: function (batch, extra) {
      extra = extra || {};
      report(
        "upload_recovered",
        "upload",
        Object.assign(
          {
            batch: String(batch || "").slice(0, 48),
            visitor_terminal_id: resolveVisitorTerminalId(),
            session_id: resolveSessionId(),
            recovered: true,
          },
          extra
        ),
        "info"
      );
    },
    resolveVisitorTerminalId: resolveVisitorTerminalId,
    resolveSessionId: resolveSessionId,
    cycleRemint: function (reason, extra) {
      var r = String(reason || "");
      // Normal navigation mint is info; only version mismatch / supersede stay warn.
      var sev =
        r === "fresh_cycle" || r === "session_storage_page_budget" || !r ? "info" : "warn";
      report(
        "cycle_remint",
        "lifecycle",
        Object.assign({ reason: r }, extra || {}),
        sev
      );
    },
    openFail: function (extra) {
      report("open_fail", "open", extra || {}, "error");
    },
    retryBudget: function (extra) {
      report("retry_budget", "retry", extra || {}, "warn");
    },
    versionUpgrade: function (selfV, serverV, action) {
      report(
        "version_upgrade",
        "self_heal",
        { self: String(selfV || ""), server: String(serverV || ""), action: String(action || "") },
        "info"
      );
    },
    /** Canonical version self-heal event (queryable: version_heal). */
    versionHeal: function (selfV, serverV, action, extra) {
      report(
        "version_heal",
        "self_heal",
        Object.assign(
          {
            self: String(selfV || ""),
            server: String(serverV || ""),
            action: String(action || ""),
          },
          extra || {}
        ),
        "info"
      );
    },
    /** Canonical vt mint with reason (queryable: vt_mint + detail.reason / vt_mint_reason). */
    vtMint: function (reason, vt, extra) {
      report(
        "vt_mint",
        "lifecycle",
        Object.assign(
          {
            reason: String(reason || "unknown"),
            vt_mint_reason: String(reason || "unknown"),
            vt: String(vt || "").slice(0, 64),
          },
          extra || {}
        ),
        "info"
      );
    },
    /** Explicit FE error/warn (call from probe code); always version-keyed. */
    feError: function (code, detail, severity) {
      report(String(code || "fe_error").slice(0, 64), "client", detail || {}, severity || "error");
    },
    /** Enable/disable client ops/error upload (panel / inject config). */
    setEnabled: function (on) {
      try {
        global.__GR_OPS_CLIENT_EVENTS__ = on ? 1 : 0;
        halted = !on;
      } catch (e) {}
    },
    isEnabled: function () {
      return clientEventsEnabled();
    },
    /** Halt uploads for this page (lifecycle complete / policy). */
    halt: function () {
      halted = true;
    },
  };
})(typeof window !== "undefined" ? window : globalThis);

/* ---- upload_queue.js ---- */
/**
 * GR UploadQueue — sub-pack self-managed upload (design from v57 upload_queue).
 * Priority sorts kick/upload order only; items never wait on each other to finish.
 */
(function (global) {
  "use strict";
  var cfg = {
    // iss/70 P2: start conservative; AIMD raises up to concurrency_max.
    concurrency: 3,
    concurrency_max: 6,
    concurrency_floor: 1,
    ramp_after_first: 4,
    /** Soft mid-ramp when B0 is only enqueued (not yet ok). */
    mid_ramp_concurrency: 4,
    apiBase: "",
    session_id: "",
    inject_path: "app",
    alive_retry_ms: 30000,
    /** Soft/mid default cap; hard anchors use lifecycle hard_max (see maxAttemptsForItem). */
    max_attempts_alive: 5,
    max_attempts_hide: 5,
    /** Hard commercial anchors (B10 / form-carrying lite) get extra priority on pagehide. */
    hard_anchor_batches: [
      "B10_hw_curves",
      "B0_bootstrap",
      "B2_hardware",
      "B3_system",
      "B1_conflict",
      "B12_anti_camouflage",
      "B8_gateway",
      "B8_gateway_early",
      // Silicon deepen: treat as pagehide-hard when residual completeness matters
      "B10x_silicon_ulp",
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
    ],
    hard_anchor_priority_boost: 1000,
    /** Extra boost after B10 lands so B10x outruns mid packs on short dwell. */
    b10x_post_b10_boost: 1500,
  };
  var pending = [];
  var inflight = 0;
  /** Items currently in postOne (for material-state / hard-SLA checks). */
  var inflightItems = [];
  /** Concurrent heavy (B10x/B7) uploads — cap 1 to reduce Failed to fetch. */
  var heavyInflight = 0;
  /** Counters for iss/70 P0/P1 observability (collect vs transport separation). */
  var transportRetryCount = 0;
  var captureFreezeCount = 0;
  var ackStored = 0;
  var ackDuplicate = 0;
  var ackMerged = 0;
  var ackConflict = 0;
  var aimdDowns = 0;
  var aimdUps = 0;
  /** Material Registry: batch_id → record (generation, hash, state, ack). */
  var materialRegistry = Object.create(null);
  /** Rolling success window for AIMD. */
  var recentUploadOk = 0;
  var recentUploadFail = 0;
  var sent = 0;
  var failed = 0;
  var skipped = 0;
  var hardFlushed = 0;
  var hardRetries = 0;
  var attempts = Object.create(null);
  var sentKeys = Object.create(null); // GA4-like: do not re-upload already-sent batch keys
  var flushReason = null;
  var hiding = false;
  var ramped = false;
  /** When set, identity/session uploads stop (cool / cycle_complete / 410). */
  var haltState = null; // { reason, session_id, code, at_ms }
  var haltedDrops = 0;
  /** In-flight AbortControllers — aborted on halt so browser does not complete more 410s. */
  var inflightCtrls = [];
  /** Session-level upload outcomes (P2 observability). */
  var sessionOutcome = {
    sealed_ok: 0,
    sealed_reject: 0,
    plain_ok: 0,
    network: 0,
    other_fail: 0,
    batches_ok: Object.create(null),
    /** Heartbeat/start markers only — must not block final payload. */
    batches_started: Object.create(null),
  };
  var outcomeReported = false;
  /** Counts soft/mid drops on pagehide and success short-circuits (ops). */
  var pagehideDropped = 0;
  var successShortCircuit = 0;
  var softAbortOnUnload = 0;
  /** force_recollect budget: max 1 per batch after terminal ok (iss/65). */
  var forceRecollectUsed = Object.create(null);

  /** Must-land silicon trio (EDH); deep is best-effort after these. */
  var B10X_MUST = ["B10x_silicon_ulp", "B10x_silicon_noderiv", "B10x_silicon_rint"];

  function batchAlreadyOk(batchId) {
    try {
      return !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok[String(batchId || "")]);
    } catch (e) {
      return false;
    }
  }

  /** Stable JSON for material hash (sorted object keys). */
  function stableStringify(v) {
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    if (Array.isArray(v)) {
      return "[" + v.map(stableStringify).join(",") + "]";
    }
    var keys = Object.keys(v).sort();
    var parts = [];
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i];
      parts.push(JSON.stringify(k) + ":" + stableStringify(v[k]));
    }
    return "{" + parts.join(",") + "}";
  }

  /** Sync SHA-256 hex (browser + node). Material identity only. */
  function sha256Hex(str) {
    try {
      if (typeof require === "function") {
        var c = require("crypto");
        if (c && c.createHash) return c.createHash("sha256").update(String(str), "utf8").digest("hex");
      }
    } catch (eN) {}
    // Minimal pure-js SHA-256 for browser lab (not crypto-grade multi-block edge; fine for dedupe).
    function rotr(n, x) {
      return (x >>> n) | (x << (32 - n));
    }
    var K = [
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
      0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
      0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
      0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
      0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
      0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
      0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
      0xc67178f2,
    ];
    function toBytes(s) {
      var utf8 = unescape(encodeURIComponent(String(s)));
      var arr = [];
      for (var i = 0; i < utf8.length; i++) arr.push(utf8.charCodeAt(i) & 255);
      return arr;
    }
    var bytes = toBytes(str);
    var l = bytes.length;
    var bitLen = l * 8;
    bytes.push(0x80);
    while ((bytes.length % 64) !== 56) bytes.push(0);
    for (var i = 7; i >= 0; i--) bytes.push((bitLen / Math.pow(2, i * 8)) & 255);
    var H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for (var off = 0; off < bytes.length; off += 64) {
      var w = new Array(64);
      for (var t = 0; t < 16; t++) {
        var j = off + t * 4;
        w[t] = (bytes[j] << 24) | (bytes[j + 1] << 16) | (bytes[j + 2] << 8) | bytes[j + 3];
      }
      for (t = 16; t < 64; t++) {
        var s0 = rotr(7, w[t - 15]) ^ rotr(18, w[t - 15]) ^ (w[t - 15] >>> 3);
        var s1 = rotr(17, w[t - 2]) ^ rotr(19, w[t - 2]) ^ (w[t - 2] >>> 10);
        w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0;
      }
      var a = H[0],
        b = H[1],
        c2 = H[2],
        d = H[3],
        e = H[4],
        f = H[5],
        g = H[6],
        h = H[7];
      for (t = 0; t < 64; t++) {
        var S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
        var ch = (e & f) ^ (~e & g);
        var t1 = (h + S1 + ch + K[t] + w[t]) | 0;
        var S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
        var maj = (a & b) ^ (a & c2) ^ (b & c2);
        var t2 = (S0 + maj) | 0;
        h = g;
        g = f;
        f = e;
        e = (d + t1) | 0;
        d = c2;
        c2 = b;
        b = a;
        a = (t1 + t2) | 0;
      }
      H[0] = (H[0] + a) | 0;
      H[1] = (H[1] + b) | 0;
      H[2] = (H[2] + c2) | 0;
      H[3] = (H[3] + d) | 0;
      H[4] = (H[4] + e) | 0;
      H[5] = (H[5] + f) | 0;
      H[6] = (H[6] + g) | 0;
      H[7] = (H[7] + h) | 0;
    }
    var out = "";
    for (i = 0; i < 8; i++) {
      var hx = (H[i] >>> 0).toString(16);
      out += ("00000000" + hx).slice(-8);
    }
    return out;
  }

  /**
   * iss/72: material hash = SHA-256 of canonical inner fields only.
   * Request-body SHA / attempt_id / seal envelope are NOT material identity.
   */
  function payloadHashOf(payload) {
    try {
      var fields = payload && payload.fields != null ? payload.fields : payload || {};
      var s = stableStringify(fields);
      return "sha256:" + sha256Hex(s);
    } catch (e) {
      return "sha256:0";
    }
  }

  /** logical key: session|batch|source|generation (iss/72) */
  function registryKey(sessionId, batchId, source, generation) {
    return (
      String(sessionId || cfg.session_id || "") +
      "|" +
      String(batchId || "") +
      "|" +
      String(source || "main") +
      "|" +
      String(generation != null ? generation : 1)
    );
  }

  function registryGet(batchId, source, sessionId, generation) {
    var sid = sessionId != null ? sessionId : cfg.session_id || "";
    var gen = generation != null ? generation : 1;
    return materialRegistry[registryKey(sid, batchId, source, gen)] || null;
  }

  function registryPut(rec) {
    if (!rec || !rec.batch_id) return;
    var sid = rec.session_id || cfg.session_id || "";
    var gen = rec.material_generation != null ? rec.material_generation : 1;
    materialRegistry[registryKey(sid, rec.batch_id, rec.source || "main", gen)] = rec;
  }

  /**
   * iss/70 Material Registry snapshot for a logical batch.
   * States: new|collected|queued|uploading|retry_wait|acked|conflict|transport_ok_unverified|absent
   */
  function registryUpsertFromItem(item, state) {
    if (!item || !item.batch_id) return null;
    var src = item.source || "main";
    var sid = item.session_id || cfg.session_id || "";
    var gen = item.material_generation != null ? item.material_generation : 1;
    var prev = registryGet(item.batch_id, src, sid, gen) || {};
    var rec = {
      batch_id: String(item.batch_id),
      source: src,
      session_id: sid,
      capture_id: item.capture_id || prev.capture_id || null,
      material_generation: gen,
      material_hash: item.payload_hash || prev.material_hash || null,
      payload_hash: item.payload_hash || prev.payload_hash || null,
      request_hash: prev.request_hash || null,
      state: state || prev.state || "collected",
      ack_type: prev.ack_type || null,
      transport_attempts: item._transport_attempts || prev.transport_attempts || 0,
      updated_ms: Date.now(),
    };
    registryPut(rec);
    return rec;
  }

  /**
   * Apply server ACK (iss/72 strict).
   * Only explicit ack.type in {stored,duplicate,merged} → ACKED.
   * Missing/empty ack → transport_ok_unverified (NOT batches_ok).
   */
  function applyAck(item, j) {
    j = j || {};
    var ack = j.ack && typeof j.ack === "object" ? j.ack : null;
    var type = ack && ack.type ? String(ack.type) : "";
    var bid = String((item && item.batch_id) || "");
    var src = (item && item.source) || "main";
    var sid = (item && item.session_id) || cfg.session_id || "";
    var gen =
      (ack && ack.material_generation != null
        ? ack.material_generation
        : item && item.material_generation != null
          ? item.material_generation
          : 1) | 0;
    // Prefer client material hash for registry identity; server body hash is request_hash.
    var materialHash =
      (ack && (ack.material_hash || ack.client_payload_hash)) ||
      (item && item.payload_hash) ||
      null;
    var requestHash = (ack && ack.payload_hash) || j.payload_hash || null;
    if (!type) {
      if (j.conflict === true || j.ack_conflict === true) type = "conflict";
      else if (j.duplicate === true || j.cold_skipped_unchanged === true) type = "duplicate";
      else if (j.merged === true) type = "merged";
      else if (j.ok === false && j.error) type = "rejected";
      else if (j.empty_body === true) type = "transport_ok_unverified";
      else if (j.ok === true && !ack) type = "transport_ok_unverified";
      else type = "transport_ok_unverified";
    }
    // Validate key match when server provides fields.
    // Explicit server type stored|duplicate|merged is authoritative (iss/72).
    // Do not reclassify server-success into conflict on minor echo mismatches
    // (sealed path may omit/reshape capture_id; material_hash remains client-side).
    var serverExplicitOk =
      type === "stored" || type === "duplicate" || type === "merged";
    if (ack && !serverExplicitOk) {
      if (ack.batch_id && String(ack.batch_id) !== bid) type = "rejected";
      if (ack.source && String(ack.source) !== src) type = "rejected";
      if (ack.session_id && String(ack.session_id) !== String(sid)) type = "rejected";
      if (
        ack.material_generation != null &&
        Number(ack.material_generation) !==
          Number(item && item.material_generation != null ? item.material_generation : 1)
      ) {
        type = "rejected";
      }
    }
    // Only force conflict when server says so (or explicit conflict flags).
    if (type !== "conflict" && (j.conflict === true || j.ack_conflict === true)) {
      type = "conflict";
    }
    var rec = registryGet(bid, src, sid, gen) || {
      batch_id: bid,
      source: src,
      session_id: sid,
      material_generation: gen,
    };
    rec.material_hash = materialHash || rec.material_hash;
    rec.payload_hash = materialHash || rec.payload_hash;
    rec.request_hash = requestHash || rec.request_hash;
    rec.capture_id = (item && item.capture_id) || rec.capture_id;
    var ingest = (j && j.ingest && typeof j.ingest === "object") ? j.ingest : {};
    var durability = String(
      (j && j.durability_state) ||
        (ack && ack.durability_state) ||
        ingest.durability_state ||
        ""
    );
    var coldWritten = j.cold_written;
    if (coldWritten === undefined) coldWritten = ingest.cold_written;
    var coldOk =
      coldWritten !== false ||
      j.cold_skipped_unchanged === true ||
      ingest.cold_skipped_unchanged === true ||
      j.same_capture === true ||
      ingest.same_capture === true ||
      durability === "duplicate_durable";
    if (
      durability === "stored_primary_only" ||
      durability === "accepted_pending_durability" ||
      durability === "failed"
    ) {
      coldOk = false;
    }
    var verified =
      (type === "stored" || type === "duplicate" || type === "merged") && coldOk;
    if (!coldOk && (type === "stored" || type === "duplicate" || type === "merged")) {
      type = "transport_ok_unverified";
    }
    rec.ack_type = type;
    rec.durability_state = durability || (coldOk ? "stored_durable" : "stored_primary_only");
    rec.updated_ms = Date.now();
    try {
      if (
        verified &&
        bid === "B11_interaction" &&
        global.GRRpaMonitor &&
        typeof global.GRRpaMonitor.ackSegment === "function"
      ) {
        global.GRRpaMonitor.ackSegment(
          src,
          (ack && ack.seq_end) || item.rpa_seq_end,
          (ack && ack.segment_id) || item.rpa_segment_id
        );
      }
    } catch (eRpaAck) {}
    if (verified) {
      rec.state = "acked";
      ackStored += type === "stored" ? 1 : 0;
      if (type === "duplicate") ackDuplicate++;
      if (type === "merged") ackMerged++;
    } else if (type === "conflict") {
      rec.state = "conflict";
      ackConflict++;
    } else if (type === "transport_ok_unverified") {
      rec.state = "transport_ok_unverified";
    } else {
      rec.state = "rejected";
    }
    registryPut(rec);
    return { type: type, verified: verified, material_hash: materialHash, request_hash: requestHash };
  }

  /** Deep-clone JSON-safe material so post-freeze collector churn cannot alias nested fields. */
  function deepCloneJson(v) {
    try {
      return JSON.parse(JSON.stringify(v));
    } catch (eDc) {
      return v;
    }
  }

  /**
   * iss/72: freeze capture at boundary — ALL material fields before hash.
   * After freeze, postOne must not mutate item.payload.
   * iss/75: deep-clone fields (shallow Object.assign left nested refs shared with collectors
   * → false `capture_mutated_blocked` on B0/B1/B10 enrich / dual-land).
   */
  function freezeCapture(item) {
    if (!item || item._capture_frozen) return item;
    try {
      var productVer =
        (global.__GR_PRODUCT_VERSION__ ||
          (global.__GR_BOOT__ &&
            (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
          "") + "";
      var payload = item.payload;
      if (!payload || typeof payload !== "object") payload = {};
      else payload = Object.assign({}, payload);
      var pf =
        payload.fields && typeof payload.fields === "object"
          ? deepCloneJson(payload.fields)
          : {};
      if (!pf || typeof pf !== "object") pf = {};
      if (productVer && !pf.product_version) pf.product_version = productVer;
      if (productVer && !payload.product_version) payload.product_version = productVer;
      // Timing once
      try {
        if (pf.t_perf == null) {
          var tPerf =
            typeof performance !== "undefined" && performance.now ? performance.now() : null;
          var tWall = Date.now();
          if (tPerf != null && isFinite(tPerf)) {
            pf.t_perf = Math.round(tPerf * 1000) / 1000;
            pf.t_wall_ms = tWall;
          }
        }
      } catch (eClk) {}
      try {
        if (global.__GR_COMPUTE_PRESSURE_STATE__ && !pf.compute_pressure_state) {
          pf.compute_pressure_state = String(global.__GR_COMPUTE_PRESSURE_STATE__);
        }
      } catch (ePr) {}
      // Cohort / FE impl — frozen INTO material before hash (iss/72 P0-3).
      try {
        if (!pf.cohort_color_gamut && typeof matchMedia === "function") {
          if (matchMedia("(color-gamut: rec2020)").matches) pf.cohort_color_gamut = "rec2020";
          else if (matchMedia("(color-gamut: p3)").matches) pf.cohort_color_gamut = "p3";
          else if (matchMedia("(color-gamut: srgb)").matches) pf.cohort_color_gamut = "srgb";
          else pf.cohort_color_gamut = "unknown";
        }
        if (pf.cohort_dpr_bucket == null && typeof devicePixelRatio === "number") {
          pf.cohort_dpr_bucket = Math.round(devicePixelRatio * 4) / 4;
        }
        if (!pf.cohort_pointer && typeof matchMedia === "function") {
          pf.cohort_pointer = matchMedia("(pointer: fine)").matches
            ? "fine"
            : matchMedia("(pointer: coarse)").matches
              ? "coarse"
              : "none";
        }
        if (!pf.cohort_intl_locale) {
          try {
            pf.cohort_intl_locale = Intl.DateTimeFormat().resolvedOptions().locale || "";
            pf.cohort_intl_calendar = Intl.DateTimeFormat().resolvedOptions().calendar || "";
            pf.cohort_intl_numbering = Intl.DateTimeFormat().resolvedOptions().numberingSystem || "";
          } catch (eIntl) {}
        }
        if (pf.cohort_speech_voices_n == null) {
          try {
            if (typeof speechSynthesis !== "undefined" && speechSynthesis.getVoices) {
              pf.cohort_speech_voices_n = (speechSynthesis.getVoices() || []).length;
            }
          } catch (eVo) {}
        }
        var fePacks = String(global.__GR_FE_PACKS_VERSION__ || global.__GR_FE_CODE_VERSION__ || "");
        if (fePacks) {
          if (!pf.fe_packs_version) pf.fe_packs_version = fePacks;
          if (!pf.fe_code_version) pf.fe_code_version = fePacks;
        }
        // Seal B10 requires fe_impl_version ∈ product epoch allowlist.
        // Prefer server product over sticky module BUILD_IMPL (v5.8.*) stamps.
        try {
          var prodEpoch =
            String(
              global.__GR_SERVER_PRODUCT_VERSION__ ||
                global.__GR_PRODUCT_VERSION__ ||
                (global.__GR_BOOT__ &&
                  (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
                ""
            ) || "";
          if (prodEpoch) {
            pf.fe_impl_version = prodEpoch;
            try {
              global.__GR_FE_IMPL_VERSION__ = prodEpoch;
            } catch (eSet) {}
          } else if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        } catch (eImpl) {
          if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        }
        if (!pf.fe_build_impl && global.__GR_BUILD_IMPL__) {
          pf.fe_build_impl = String(global.__GR_BUILD_IMPL__);
        }
      } catch (eCoh) {}
      payload.fields = pf;
      item.payload = payload;
      if (!item.capture_id) {
        item.capture_id =
          "cap_" +
          String(item.batch_id || "b") +
          "_" +
          String(Date.now()) +
          "_" +
          String(Math.floor(Math.random() * 1e6));
      }
      if (item.material_generation == null) item.material_generation = 1;
      item.collected_at_ms = item.collected_at_ms || Date.now();
      item.payload_hash = payloadHashOf(payload);
      item.material_hash = item.payload_hash;
      item._capture_frozen = true;
      // Deep-freeze snapshot for retry identity checks + restore on post-freeze mutation.
      try {
        item._frozen_fields = deepCloneJson(payload.fields || {});
        item._frozen_payload_json = stableStringify(item._frozen_fields || {});
      } catch (eSnap) {}
      captureFreezeCount++;
      registryUpsertFromItem(item, "collected");
    } catch (eFz) {
      try {
        item._capture_frozen = true;
      } catch (e2) {}
    }
    return item;
  }

  /** Reset queue/registry state when session_id changes (iss/72 P1-2). */
  function resetSessionState(reason) {
    materialRegistry = Object.create(null);
    sessionOutcome = {
      sealed_ok: 0,
      sealed_reject: 0,
      plain_ok: 0,
      network: 0,
      other_fail: 0,
      batches_ok: Object.create(null),
      batches_started: Object.create(null),
    };
    forceRecollectUsed = Object.create(null);
    sentKeys = Object.create(null);
    attempts = Object.create(null);
    outcomeReported = false;
    try {
      global.__GR_RECEIVED_BATCHES__ = [];
    } catch (eR) {}
    // Drop pending from other sessions
    try {
      var keep = [];
      for (var i = 0; i < pending.length; i++) {
        if (pending[i] && String(pending[i].session_id || "") === String(cfg.session_id || "")) {
          keep.push(pending[i]);
        }
      }
      pending = keep;
    } catch (eP) {}
    try {
      if (global.GROps && GROps.report) {
        GROps.report(
          "queue_session_reset",
          "upload",
          { reason: String(reason || "session_change") },
          "info"
        );
      }
    } catch (eO) {}
  }

  /**
   * iss/70 P2: effective upload concurrency = min of budgets (never max-stack).
   */
  function effectiveUploadCap() {
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var hardMax = Math.max(floor, Number(cfg.concurrency_max) || 6);
    var base = Math.max(floor, Number(cfg.concurrency) || 3);
    // Lifecycle: unloading → 1–2; background → low.
    var lifeCap = hardMax;
    try {
      if (isPageUnloading() || hiding || global.__GR_PAGE_UNLOADING__) lifeCap = 2;
      else if (global.__GR_PAGE_BACKGROUNDED__ || isPageBackgrounded()) lifeCap = 2;
    } catch (eL) {}
    // Pressure: serious/critical → floor.
    try {
      var st = String(global.__GR_COMPUTE_PRESSURE_STATE__ || "").toLowerCase();
      if (st === "serious" || st === "critical") lifeCap = Math.min(lifeCap, floor);
    } catch (eP) {}
    // Fail storm: half.
    if (recentUploadFail >= 3 && recentUploadOk < recentUploadFail) {
      lifeCap = Math.min(lifeCap, Math.max(floor, Math.floor(base / 2) || floor));
    }
    return Math.max(floor, Math.min(hardMax, base, lifeCap));
  }

  /** AIMD: success window +1 (cap max); fail → half. */
  function aimdOnSuccess() {
    recentUploadOk++;
    recentUploadFail = Math.max(0, recentUploadFail - 1);
    var maxC = Math.max(1, Number(cfg.concurrency_max) || 6);
    if (recentUploadOk >= 3 && recentUploadFail === 0) {
      var next = Math.min(maxC, (Number(cfg.concurrency) || 3) + 1);
      if (next > cfg.concurrency) {
        cfg.concurrency = next;
        aimdUps++;
      }
      recentUploadOk = 0;
    }
  }

  function aimdOnFail() {
    recentUploadFail++;
    recentUploadOk = 0;
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var cur = Number(cfg.concurrency) || 3;
    var next = Math.max(floor, Math.floor(cur / 2) || floor);
    if (next < cur) {
      cfg.concurrency = next;
      aimdDowns++;
    }
  }

  /** True if this batch has a capture in pending/retry/upload (not yet batches_ok). */
  function hasPendingCapture(batchId, sessionId) {
    var bid = String(batchId || "");
    if (!bid) return false;
    var sid = sessionId != null ? String(sessionId) : "";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      return true;
    }
    var i;
    for (i = 0; i < pending.length; i++) if (match(pending[i])) return true;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return true;
    return false;
  }

  /**
   * Material/transport state for hard-SLA, registry, and tests.
   * acked | conflict | uploading | retry_wait | queued | collected | absent
   */
  function materialState(batchId, sessionId, source, generation) {
    var bid = String(batchId || "");
    if (!bid) return "absent";
    if (batchAlreadyOk(bid)) return "acked";
    var sid = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
    var src = source || "main";
    var gen = generation != null ? generation : 1;
    var rec = registryGet(bid, src, sid, gen);
    if (rec && rec.state === "acked") return "acked";
    if (rec && rec.state === "conflict") return "conflict";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      if (src && it.source && String(it.source) !== String(src)) return false;
      return true;
    }
    var i;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return "uploading";
    for (i = 0; i < pending.length; i++) {
      if (!match(pending[i])) continue;
      if (pending[i]._retry_after_ms && pending[i]._retry_after_ms > Date.now()) return "retry_wait";
      return "queued";
    }
    if (rec && rec.state) return rec.state;
    return "absent";
  }

  function removeInflightItem(item) {
    var kWant = "";
    try {
      kWant = keyOf(item);
    } catch (eK) {
      kWant = "";
    }
    var removed = 0;
    for (var i = inflightItems.length - 1; i >= 0; i--) {
      var cur = inflightItems[i];
      if (cur === item) {
        inflightItems.splice(i, 1);
        removed++;
        continue;
      }
      // Identity can diverge after freeze/clone — also drop by material key.
      try {
        if (kWant && keyOf(cur) === kWant) {
          inflightItems.splice(i, 1);
          removed++;
        }
      } catch (e2) {}
    }
    // Own the counter: callers must not also inflight-- after this (was double-
    // decrementing → negative inflight under multi-tab / multi-key prune).
    if (removed > 0) inflight = Math.max(0, inflight - removed);
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (inflight < 0) inflight = 0;
  }

  /** Drop stuck inflight slots so multi-tab never accumulates zombie transport rows. */
  function pruneStaleInflight() {
    var cap = Math.max(Number(cfg.concurrency_max) || 6, Number(cfg.concurrency) || 3) * 2;
    if (inflightItems.length <= cap && inflight <= cap) return;
    // Hard clamp: keep newest cap items, drop older zombies.
    if (inflightItems.length > cap) {
      inflightItems = inflightItems.slice(-cap);
    }
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (heavyInflight > 1) heavyInflight = 1;
  }

  function isB10xMustLand(batchId) {
    var id = String(batchId || "");
    for (var i = 0; i < B10X_MUST.length; i++) if (B10X_MUST[i] === id) return true;
    return false;
  }

  /**
   * pagehide / unload allowlist — only mint-critical + secondary silicon.
   * Mid/dense/R/B11 are dropped so keepalive window is not wasted (iss/65).
   */
  function isPagehideAllowlisted(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    if (id.indexOf("B10x_silicon_") === 0) {
      // On unload: prefer must-land; deep only if must-land already ok or nothing else pending.
      if (id === "B10x_silicon_deep") {
        if (!isPageUnloading()) return true;
        var mustPending = false;
        for (var i = 0; i < B10X_MUST.length; i++) {
          if (!batchAlreadyOk(B10X_MUST[i])) {
            // still need must — allow deep only if already inflight material not required
            mustPending = true;
            break;
          }
        }
        // If must still missing, drop deep on pagehide (save slots for ulp/rint/noderiv).
        return !mustPending;
      }
      return true;
    }
    if (
      id === "B0_bootstrap" ||
      id === "B2_hardware" ||
      id === "B3_system" ||
      id === "B1_conflict" ||
      id === "B12_anti_camouflage" ||
      id === "B8_gateway" ||
      id === "B8_gateway_early" ||
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B7_sandbox"
    ) {
      return true;
    }
    return false;
  }

  /**
   * Pagehide / flush priority: commercial hard + secondary silicon (B10x/B47/B18).
   * NOTE: do NOT use this alone for ops severity — B10x/B47 network exhaust must
   * report as deepen/soft exhausted (warn), not upload_hard_exhausted (error).
   * Prod v150: B10x_deep / B47 mis-tagged as hard_exhausted on Failed to fetch.
   */
  function isHardAnchorBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    // Secondary silicon/infra: hard *priority* on pagehide only (not always-hard).
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    ) {
      return isPageUnloading() || !!sessionOutcome.batches_ok["B10_hw_curves"];
    }
    // B10x: hard on pagehide / when B10 already ok (short-visit residual path)
    if (id.indexOf("B10x_silicon_") === 0) {
      if (isPageUnloading()) return true;
      try {
        if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) return true;
      } catch (eB) {}
      return false;
    }
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    // alias mid.curves / primary silicon residual only
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    return false;
  }

  /**
   * Ops severity gate: only true commercial anchors → upload_hard_exhausted error.
   * Silicon deepen / SAB / WebGPU network storms → deepen_exhausted warn.
   */
  function isCommercialHardForOps(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return false;
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B15_cross_curves"
    ) {
      return false;
    }
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    return false;
  }

  function isDeepenBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isDeepenBatch) {
        return !!GRProbeLifecycle.isDeepenBatch(batchId);
      }
    } catch (eD) {}
    var id = String(batchId || "");
    return id.indexOf("B10x_") === 0 || id === "B15_cross_curves";
  }

  function isHeavyBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isHeavyBatch) {
        return !!GRProbeLifecycle.isHeavyBatch(batchId);
      }
    } catch (eH) {}
    var id = String(batchId || "");
    return (
      isDeepenBatch(id) ||
      id === "B7_sandbox" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep"
    );
  }

  function heavyMaxInflight() {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.heavyMaxInflight) {
        return Math.max(1, GRProbeLifecycle.heavyMaxInflight() | 0);
      }
      if (global.GRProbeLifecycle && GRProbeLifecycle.POLICY) {
        return Math.max(1, (GRProbeLifecycle.POLICY.heavy_max_inflight || 1) | 0);
      }
    } catch (eM) {}
    return 1;
  }

  function isStartHeartbeatItem(item) {
    try {
      var f =
        (item && item.payload && item.payload.fields) ||
        (item && item.fields) ||
        null;
      if (!f || typeof f !== "object") return false;
      if (String(f.b10x_phase || "") === "start") return true;
      if (String(f.b10x_err || "") === "started") return true;
      if (String(f.b10x_err || "") === "helpers_missing") return true;
      return false;
    } catch (e) {
      return false;
    }
  }

  function noteOutcome(kind, batchId, item) {
    try {
      if (kind === "sealed_ok") sessionOutcome.sealed_ok++;
      else if (kind === "sealed_reject") sessionOutcome.sealed_reject++;
      else if (kind === "plain_ok") sessionOutcome.plain_ok++;
      else if (kind === "network") sessionOutcome.network++;
      else sessionOutcome.other_fail++;
      if (batchId && (kind === "sealed_ok" || kind === "plain_ok")) {
        var bid = String(batchId);
        // Start/heartbeat must not count as terminal success (blocks final multipath).
        if (isStartHeartbeatItem(item)) {
          sessionOutcome.batches_started[bid] = 1;
          return;
        }
        sessionOutcome.batches_ok[bid] = 1;
        // After B10 lands, boost pending B10x so residual path wins short dwell.
        if (bid === "B10_hw_curves") {
          try {
            boostPendingB10x();
          } catch (eBoost) {}
        }
      }
    } catch (eO) {}
  }

  function boostPendingB10x() {
    var boost = Number(cfg.b10x_post_b10_boost || 1500);
    for (var i = 0; i < pending.length; i++) {
      var it = pending[i];
      if (!it) continue;
      var bid = String(it.batch_id || "");
      if (bid.indexOf("B10x_silicon_") === 0) {
        it.priority = Math.max(Number(it.priority || 0), boost);
      }
    }
    try {
      pending.sort(function (a, b) {
        return Number(b.priority || 0) - Number(a.priority || 0);
      });
    } catch (eSrt) {}
    pump();
  }

  /** P2: one session summary for ops (gateway_only vs browser_probe). */
  function reportSessionUploadSummary(force) {
    if (outcomeReported && !force) return;
    try {
      var okKeys = Object.keys(sessionOutcome.batches_ok || {});
      var hasB0 = !!sessionOutcome.batches_ok["B0_bootstrap"];
      var hasB10 = !!sessionOutcome.batches_ok["B10_hw_curves"];
      var hasB8 =
        !!sessionOutcome.batches_ok["B8_gateway"] ||
        !!sessionOutcome.batches_ok["B8_gateway_early"];
      var hasB10x = false;
      for (var k = 0; k < okKeys.length; k++) {
        if (String(okKeys[k]).indexOf("B10x_silicon_") === 0) {
          hasB10x = true;
          break;
        }
      }
      var onlyGw = hasB8 && !hasB0 && okKeys.length <= 2;
      var depthClass = onlyGw
        ? "gateway_only"
        : hasB0 && hasB10 && hasB10x
          ? "browser_b0_b10_silicon"
          : hasB0 && hasB10
            ? "browser_b0_b10"
            : hasB0
              ? "browser_partial"
              : okKeys.length
                ? "browser_lite"
                : "empty";
      if (global.GROps && GROps.report) {
        outcomeReported = true;
        GROps.report(
          "upload_session_summary",
          "upload",
          {
            sealed_ok: sessionOutcome.sealed_ok,
            sealed_reject: sessionOutcome.sealed_reject,
            plain_ok: sessionOutcome.plain_ok,
            network: sessionOutcome.network,
            other_fail: sessionOutcome.other_fail,
            batches_ok_n: okKeys.length,
            has_b0: hasB0,
            has_b10: hasB10,
            has_b10x: hasB10x,
            probe_depth_class: depthClass,
          },
          "info"
        );
      }
    } catch (eS) {}
  }

  /** True only on real unload — tab background must NOT set this. */
  function isPageUnloading() {
    try {
      return !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_UNLOADING__);
    } catch (e) {
      return false;
    }
  }

  function isPageBackgrounded() {
    try {
      return !!global.__GR_PAGE_BACKGROUNDED__;
    } catch (e) {
      return false;
    }
  }

  function effectivePriority(item) {
    var p = item.priority || 0;
    var bid = String((item && item.batch_id) || "");
    // Always prefer commercial land + early caps over soft-pack storms (iss/75 texture race).
    if (bid === "B10_hw_curves" || bid === "mid.curves") p += 2200;
    else if (bid === "B2_hardware") p += 1800;
    else if (isCommercialLandFast(bid)) p += 1600;
    if (hiding && isHardAnchorBatch(bid)) {
      p += cfg.hard_anchor_priority_boost || 1000;
    }
    // Active maximize path (alive dwell): keep secondary silicon/infra + B10x above soft
    // media force storms (B19 dense hedge / client_hints). Aligns with probe_field_priority
    // silicon_p0 → media_p1/infra_p1 without blocking soft packs from eventually uploading.
    // Observed: Brave matrix dwell ended pending=250, B19×35, missing B47/B18/B46.
    if (!hiding && !isPageUnloading()) {
      var b10Ok = false;
      try {
        b10Ok = !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]);
      } catch (eB10) {}
      if (isB10xBatch(bid)) {
        p += b10Ok ? cfg.b10x_post_b10_boost || 1500 : 900;
      } else if (isSecondaryInfraBatch(bid)) {
        // B47/B18/B46/B10x_deep: must not sit behind soft force re-queues mid-dwell.
        p += b10Ok ? 1200 : 700;
      }
    }
    // Unload: must-land B10x above deep / secondary so short keepalive lands silicon.
    if (hiding || isPageUnloading()) {
      if (isB10xMustLand(bid)) p += 2500;
      else if (bid === "B10_hw_curves" || bid === "mid.curves") p += 3000;
      else if (bid === "B10x_silicon_deep") p -= 400;
      else if (!isPagehideAllowlisted(bid)) p -= 5000;
    }
    return p;
  }

  /**
   * Insert or replace pending by session|batch|source key.
   * Force items MUST coalesce (was: force=true skipped replace → N copies of B19/secondary).
   * Does not change wire semantics: still one transport path per key at a time.
   * @returns {"pushed"|"replaced"|"dropped"}
   */
  function queuePending(item) {
    if (!item || !item.batch_id) return "dropped";
    var k = keyOf(item);
    var genNew = (item.material_generation || 1) | 0;
    for (var i = 0; i < pending.length; i++) {
      if (keyOf(pending[i]) !== k) continue;
      var cur = pending[i];
      var genCur = (cur.material_generation || 1) | 0;
      // Never demote a higher material generation already queued.
      if (genNew < genCur) {
        skipped++;
        return "dropped";
      }
      // Same key: replace with fresher capture (force or not). Counts as dedupe.
      pending[i] = item;
      skipped++;
      return "replaced";
    }
    pending.push(item);
    return "pushed";
  }

  function keyOf(item) {
    return [item.session_id || cfg.session_id, item.batch_id, item.source || "main"].join("|");
  }

  function sessionOf(item) {
    return String(
      (item && item.session_id) || cfg.session_id || global.__GR_SESSION_ID__ || ""
    );
  }

  /** Batches that must stop when cycle is complete / cool (identity path). */
  function isIdentityBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return true;
    // Page RPA after cool still uses B11 in some designs — block on hard halt too
    // because server require_active_session rejects ALL ingest on complete cycle.
    return true;
  }

  /**
   * Terminal = this cycle must stop identity uploads.
   * Align with BE:
   *   - 410 / cycle_closed / halt_uploads / analysis_terminal / cycle_complete → halt
   *   - soft probe_complete alone → NOT halt (still may upload rest packs)
   *   - open cool skip_identity → halt identity
   */
  /**
   * Schedule-final complete (maximize probe policy v5.8.53+).
   * True only when brain schedule is done AND silicon/B10 materials exist.
   * commercial_identity_final / dh_+curves alone is a milestone — NOT final.
   */
  function hardFinalComplete(j, cps) {
    cps = cps || {};
    var cov = cps.identity_coverage || j.identity_coverage || {};
    // EDH: block cool while silicon B10x still required/missing.
    try {
      var edh = j.edh || j.device_hypothesis || {};
      var rg = edh.research_gate || {};
      if (rg.b10x_required === true && rg.b10x_complete === false) return false;
      if (j.b10x_must_land === true) return false;
      var miss = (j.route_plan && j.route_plan.b10x_missing) || (j.b10x_missing) || [];
      if (miss && miss.length) return false;
      var rp = j.route_plan || (j.brain && j.brain.route_plan) || {};
      var packs = rp.packs || [];
      for (var pi = 0; pi < packs.length; pi++) {
        var pid = String((packs[pi] && (packs[pi].pack_id || packs[pi].id)) || "");
        if (pid.indexOf("B10x_silicon_") === 0) return false;
      }
    } catch (eEdh) {}

    // Primary residual B10 MUST land before any cool/final — do not trust digest alone.
    var ids = cps.received_batch_ids || j.received_batch_ids || [];
    var hasB10 = false;
    for (var i = 0; i < ids.length; i++) {
      var id = typeof ids[i] === "string" ? ids[i] : ids[i] && ids[i].batch_id;
      if (id === "B10_hw_curves" || id === "mid.curves") {
        hasB10 = true;
        break;
      }
    }
    try {
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) hasB10 = true;
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]) hasB10 = true;
    } catch (eLoc) {}
    try {
      if (j.b10_present === true && (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"])) {
        hasB10 = true;
      }
    } catch (eB) {}
    // Without primary B10, never final (keeps self-heal / upload alive).
    if (!hasB10) return false;

    // commercial_identity_final alone → NOT final (continue soft/mid packs).
    var coverageComplete =
      cov.coverage_complete === true ||
      cps.coverage_complete === true ||
      j.coverage_complete === true ||
      (j.coverage && j.coverage.coverage_complete === true) ||
      (j.brain && j.brain.coverage && j.brain.coverage.coverage_complete === true);
    var stopProbe =
      j.stop_probe === true ||
      (j.route_plan && j.route_plan.stop_probe === true) ||
      (cps.probe_complete === true && coverageComplete);
    var brainTerm =
      j.analysis_terminal === true ||
      cps.analysis_terminal === true ||
      (j.analysis && j.analysis.analysis_terminal === true) ||
      (stopProbe && coverageComplete);
    var closed =
      cps.cycle_status === "complete" ||
      j.cycle_complete === true ||
      cps.cycle_closed === true ||
      j.cycle_closes === true ||
      (j.analysis && (j.analysis.cycle_closes === true || j.analysis.cycle_complete === true));
    var scheduleFinal =
      cov.brain_schedule_final === true ||
      cps.brain_schedule_final === true ||
      j.brain_schedule_final === true ||
      cov.final_analysis_ok === true ||
      cps.final_analysis_ok === true ||
      j.final_analysis_ok === true;

    // Final = primary B10 landed AND (schedule done OR closed OR hard_complete).
    if (scheduleFinal) return true;
    if (brainTerm || closed) return true;
    if (cov.hard_complete === true) return true;
    return false;
  }

  function parseTerminal(status, j) {
    j = j || {};
    var err = j.error || j.code || j.expired_reason || "";
    var code = j.code || j.expired_reason || "";
    var cps = j.cycle_probe_status || {};
    var analysis = j.analysis || {};
    var s =
      String(err) +
      " " +
      String(code) +
      " " +
      String(cps.cycle_status || "") +
      " " +
      String(cps.business_state || "") +
      " " +
      String(cps.expired_reason || "");

    // Soft cycle close (v5.8.124): HTTP 200 + accepted:false + halt_uploads.
    // Preferred over 410 so browser Network is not full of red errors.
    if (
      status === 200 &&
      j &&
      (j.accepted === false ||
        j.identity_accepted === false ||
        (j.halt_uploads === true &&
          (j.cycle_closed === true ||
            j.code === "cycle_complete" ||
            j.code === "cycle_purged" ||
            j.code === "incomplete_ttl" ||
            j.code === "session_expired" ||
            (j.cycle_probe_status && j.cycle_probe_status.http_soft_close))))
    ) {
      return {
        terminal: true,
        code: code || j.code || cps.expired_reason || "cycle_complete",
        status: 200,
        business_state: j.business_state || cps.business_state || "identity_complete_cool",
        soft_close: true,
      };
    }

    // Legacy HTTP 410: cycle no longer accepts identity ingest (complete|purged|ttl).
    if (status === 410) {
      return {
        terminal: true,
        code: code || cps.expired_reason || "session_expired",
        status: 410,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }

    // Authoritative hard+final complete (probe completeness + final analysis).
    if (hardFinalComplete(j, cps)) {
      return {
        terminal: true,
        code: code || "identity_final_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // halt_uploads from server only if hard complete OR cycle truly closed with hard materials.
    if (j.halt_uploads === true || cps.halt_uploads === true || analysis.halt_uploads === true) {
      if (!hardFinalComplete(j, cps) && cps.cycle_status !== "purged" && status !== 410) {
        // Thin halt flag — keep uploading B10 (server may lag; hard SLA continues).
        return { terminal: false };
      }
      var haltCode =
        code ||
        (analysis.cycle_complete || j.cycle_complete
          ? "cycle_complete"
          : cps.expired_reason || cps.cycle_status || cps.business_state || "halt_uploads");
      return {
        terminal: true,
        code: haltCode,
        status: status || 200,
        business_state: cps.business_state || "halt_uploads",
      };
    }

    // Cycle closed flags — only halt when hard materials present (or purged).
    if (
      j.cycle_closes === true ||
      j.cycle_complete ||
      cps.cycle_closed === true ||
      cps.cycle_status === "complete" ||
      cps.cycle_status === "purged" ||
      analysis.cycle_complete ||
      analysis.cycle_closes === true
    ) {
      if (cps.cycle_status === "purged") {
        return {
          terminal: true,
          code: "cycle_purged",
          status: status || 200,
          business_state: "purged",
        };
      }
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cycle_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // Soft analysis_terminal without B10: keep probing (do not halt).
    if (
      j.analysis_terminal === true ||
      analysis.analysis_terminal === true ||
      cps.analysis_terminal === true
    ) {
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: "analysis_terminal",
        status: status || 200,
        business_state: "analysis_terminal",
      };
    }

    // Cool / open skip identity — ONLY when phase/business_state says cool AND silicon ok.
    if (
      j.phase === "cool" ||
      cps.business_state === "identity_complete_cool" ||
      (j.skip_identity_probe === true &&
        (j.phase === "cool" ||
          cps.cycle_status === "complete" ||
          cps.halt_uploads === true ||
          j.halt_uploads === true)) ||
      (j.skip_session_probe === true &&
        (j.phase === "cool" || cps.business_state === "identity_complete_cool"))
    ) {
      if (j.cool_silicon_ok === false || cps.cool_silicon_ok === false) {
        return { terminal: false };
      }
      // Prefer hard materials when available; thin cool already blocked server-side.
      if (!hardFinalComplete(j, cps) && j.cool_silicon_ok !== true && cps.has_b10 !== true) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cool",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    if (/session_expired|cycle_purged|incomplete_ttl/i.test(s)) {
      return {
        terminal: true,
        code: code || "session_expired",
        status: status || 0,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }
    // cycle_complete / halt_uploads in error string alone must not stop thin uploads.
    return { terminal: false };
  }

  function maybeLocalIdentityDone() {
    // Local channel only: FE-side upload progress. Does NOT close the cycle —
    // server analysis_terminal / complete_cycle / 410 remain authoritative for halt.
    var need = cfg.hard_anchor_batches || [];
    var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
    var ok = 0;
    var present = [];
    for (var i = 0; i < need.length; i++) {
      var bid = need[i];
      if (bid === "B8_gateway_early") continue;
      var k1 = [sid, bid, "main"].join("|");
      var k2 = bid === "B8_gateway" ? [sid, "B8_gateway_early", "main"].join("|") : null;
      if (sentKeys[k1] || (k2 && sentKeys[k2])) {
        ok++;
        present.push(bid);
      }
    }
    var idle = pending.length === 0 && inflight <= 1;
    // B0+B1+B2+B3+B12+B8 ≈ 6 without B10 is enough for "lite wave uploaded" local signal
    if (ok >= 5 && idle) {
      try {
        global.__GR_IDENTITY_UPLOADS_DONE__ = {
          at_ms: Date.now(),
          hard_sent: ok,
          present: present,
          session_id: sid,
          // Soft local progress — not cycle closed
          local_only: true,
          cycle_closed: !!(haltState || global.__GR_CYCLE_CLOSED__),
        };
        global.dispatchEvent(
          new CustomEvent("gr-identity-uploads-done", {
            detail: global.__GR_IDENTITY_UPLOADS_DONE__,
          })
        );
      } catch (eD) {}
    }
    // Queue fully idle: notify multi-tick / observers (not halt).
    if (pending.length === 0 && inflight === 0) {
      try {
        global.__GR_UPLOAD_QUEUE_IDLE__ = {
          at_ms: Date.now(),
          sent: sent,
          session_id: sid,
          halted: !!haltState,
        };
        global.dispatchEvent(
          new CustomEvent("gr-upload-queue-idle", {
            detail: global.__GR_UPLOAD_QUEUE_IDLE__,
          })
        );
      } catch (eI) {}
    }
  }

  function phaseForCode(code, reason) {
    var c = String(code || reason || "");
    if (/incomplete_ttl|purged|cycle_purged/i.test(c)) return "expired";
    if (/cool|cycle_complete|analysis_terminal|halt|probe_complete|stop_probe/i.test(c)) return "cool";
    return "cool";
  }

  function applyHalt(reason, sessionId, code) {
    var sid = String(sessionId || cfg.session_id || global.__GR_SESSION_ID__ || "");
    var c = code || reason || "halt";
    // Idempotent: same session already halted — still abort leftovers.
    try {
      reportSessionUploadSummary(true);
    } catch (eSum) {}
    var cLow0 = String(c || reason || "").toLowerCase();
    // Soft-close preferred: cycle complete / cool are product-normal (HTTP 200), not 410.
    var softTerminal =
      cLow0.indexOf("cycle_complete") >= 0 ||
      cLow0.indexOf("identity_complete") >= 0 ||
      cLow0.indexOf("identity_final") >= 0 ||
      cLow0 === "cool" ||
      cLow0 === "probe_complete" ||
      cLow0 === "analysis_terminal" ||
      cLow0 === "incomplete_ttl" ||
      cLow0 === "cycle_purged" ||
      cLow0 === "session_expired";
    haltState = {
      reason: reason || "halt",
      session_id: sid,
      code: c,
      at_ms: Date.now(),
      business_state: phaseForCode(c, reason),
      status: softTerminal ? 200 : 410,
      soft_close: !!softTerminal,
    };
    try {
      global.__GR_STOP_PROBE__ = true;
      global.__GR_SKIP_IDENTITY__ = true;
      global.__GR_HALT_UPLOADS__ = true;
      global.__GR_PHASE__ = phaseForCode(c, reason);
      global.__GR_UPLOAD_HALT__ = haltState;
      global.__GR_CYCLE_CLOSED__ = {
        session_id: sid,
        code: c,
        reason: reason,
        at_ms: haltState.at_ms,
        soft_close: !!softTerminal,
      };
      // Persist cool ONLY on true schedule-final / identity_complete / cycle_complete cool.
      // Cool is version-scoped (product_version stamp); version change invalidates via storage.
      // Do NOT stamp 24h cool on generic superseded/pagehide — that froze incomplete VT gap-fill.
      try {
        var S = global.GRStorage;
        var cLow = String(c || reason || "").toLowerCase();
        var coolOkHalt =
          cLow.indexOf("identity_complete") >= 0 ||
          cLow.indexOf("identity_final") >= 0 ||
          cLow.indexOf("schedule_final") >= 0 ||
          cLow.indexOf("cycle_complete") >= 0 ||
          cLow === "cool" ||
          cLow === "probe_complete" ||
          cLow === "analysis_terminal" ||
          !!global.__GR_BRAIN_SCHEDULE_FINAL__ ||
          !!global.__GR_HARD_FINAL__;
        // pagehide / superseded without schedule final → no cool stamp
        if (
          S &&
          S.setCoolUntil &&
          coolOkHalt &&
          !isPageUnloading()
        ) {
          var until = S.getCoolUntil && S.getCoolUntil();
          var now = Date.now();
          var ver =
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            (global.__GR_BOOT__ && (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
            global.__GR_PRODUCT_VERSION__ ||
            (haltState && haltState.product_version) ||
            undefined;
          if (!until || until <= now) {
            S.setCoolUntil(now + 24 * 60 * 60 * 1000, ver);
          } else {
            // Refresh version stamp on existing cool window (keeps cool tied to current version).
            S.setCoolUntil(until, ver);
          }
        }
      } catch (eCool) {}
    } catch (eG) {}
    // Drop pending except B10x EDH must-land (and explicit force_after_halt).
    if (pending.length) {
      var keepB10x = [];
      for (var pi = 0; pi < pending.length; pi++) {
        if (allowDespiteHalt(pending[pi])) keepB10x.push(pending[pi]);
        else haltedDrops++;
      }
      pending = keepB10x;
    }
    if (!pending.length) {
      cfg.concurrency = 0;
    } else {
      // Keep a small pump capacity so silicon B10x can finish after identity halt.
      if (cfg.concurrency < 2) cfg.concurrency = 2;
      try {
        setTimeout(function () {
          pump();
        }, 0);
      } catch (eP) {}
    }
    // Abort in-flight fetch so concurrent POSTs do not all land as 410 in Network.
    var ctrls = inflightCtrls.slice();
    inflightCtrls = [];
    for (var ai = 0; ai < ctrls.length; ai++) {
      try {
        if (ctrls[ai] && ctrls[ai].abort) ctrls[ai].abort();
      } catch (eAb) {}
    }
    try {
      global.dispatchEvent(
        new CustomEvent("gr-cycle-closed", {
          detail: {
            reason: haltState.reason,
            code: haltState.code,
            session_id: haltState.session_id,
          },
        })
      );
    } catch (eEv) {}
  }

  function isHaltedFor(item) {
    try {
      if (global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) return true;
    } catch (eH) {}
    if (!haltState) return false;
    var sid = sessionOf(item);
    if (!haltState.session_id) return true;
    if (!sid) return true;
    return sid === haltState.session_id;
  }

  /** EDH silicon deepen must still upload after identity halt / cool. */
  function isB10xBatch(batchId) {
    return String(batchId || "").indexOf("B10x_") === 0;
  }

  /** iss/54–57 secondary silicon/infra — must land ok or honest skip even after identity halt. */
  function isSecondaryInfraBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    );
  }

  /**
   * Re-open cycle to remint seal_grant (open lag / wiped grant).
   * Module-level so enqueue + postOne share one coalesced refresh.
   */
  function refreshSealGrant(sid) {
    if (global.__GR_SEAL_REFRESH_P__) return global.__GR_SEAL_REFRESH_P__;
    var base = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "");
    var cycle = String(
      sid ||
        global.__GR_SESSION_ID__ ||
        global.__GR_CYCLE_ID__ ||
        cfg.session_id ||
        ""
    );
    var vt = "";
    try {
      vt =
        global.__GR_VTID__ ||
        (global.GRStorage && GRStorage.visitorTerminalId && GRStorage.visitorTerminalId()) ||
        "";
    } catch (eV) {}
    var site =
      global.__GR_SITE_ID__ ||
      (global.__GR_BOOT__ && (global.__GR_BOOT__.site_id || global.__GR_BOOT__.siteId)) ||
      "";
    // embed gate: open 需带站点 embed_token。优先 __GR_BOOT__.embed_token，
    // 否则从 pin_url / 含 grt= 的 script 标签取(与 gr.boot embedTokenFromUrl 同源)。
    var etok = "";
    try {
      var bc = global.__GR_BOOT__ || {};
      etok = String(bc.embed_token || "");
      if (!etok) {
        var srcs = [];
        if (bc.pin_url) srcs.push(String(bc.pin_url));
        try {
          if (typeof document !== "undefined" && document.querySelectorAll) {
            var tags = document.querySelectorAll('script[src*="grt="]');
            for (var i = 0; i < tags.length; i++) srcs.push(String(tags[i].src || ""));
          }
        } catch (eQs0) {}
        for (var j = 0; j < srcs.length; j++) {
          var m = srcs[j].match(/[?&]grt=([^&]+)/);
          if (m) {
            etok = decodeURIComponent(m[1]);
            break;
          }
        }
      }
    } catch (eEt) {}
    var openUrl = base + "/v1/session/open";
    var openSame = false;
    try {
      openSame =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(openUrl, location.href).origin === location.origin;
    } catch (eOpenSo) {
      openSame = String(openUrl).charAt(0) === "/";
    }
    global.__GR_SEAL_REFRESH_P__ = fetch(openUrl, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        cycle_id: cycle || undefined,
        session_id: cycle || undefined,
        visitor_terminal_id: vt || undefined,
        site_id: site || undefined,
        embed_token: etok || undefined,
        meta: { fe: "seal_refresh", inject_path: cfg.inject_path || "app" },
      }),
      // CF orange: same-origin must send cf_clearance; cross-origin include when CORS allows
      credentials: openSame ? "same-origin" : "include",
      mode: openSame ? "same-origin" : "cors",
      cache: "no-store",
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        if (j && j.seal_grant) {
          try {
            global.__GR_SEAL_GRANT__ = j.seal_grant;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.seal_grant = j.seal_grant;
            if (j.require_sealed_ingest) {
              global.__GR_REQUIRE_SEALED__ = true;
              global.__GR_BOOT__.require_sealed_ingest = true;
            }
            if (global.GRSeal) {
              if (GRSeal.setRequireSealed && j.require_sealed_ingest) GRSeal.setRequireSealed(true);
              if (GRSeal.setGrant) GRSeal.setGrant(j.seal_grant);
              if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            }
            var ns = j.cycle_id || j.session_id || "";
            if (ns) {
              global.__GR_SESSION_ID__ = ns;
              global.__GR_CYCLE_ID__ = ns;
              cfg.session_id = ns;
            }
          } catch (eG) {}
          return true;
        }
        return false;
      })
      .catch(function () {
        return false;
      })
      .then(function (ok) {
        global.__GR_SEAL_REFRESH_P__ = null;
        try {
          if (ok) pump();
        } catch (eK) {}
        return ok;
      });
    return global.__GR_SEAL_REFRESH_P__;
  }

  function allowDespiteHalt(item) {
    if (!item) return false;
    if (item.force_after_halt || item.allow_during_stop) return true;
    if (isB10xBatch(item.batch_id)) return true;
    if (isSecondaryInfraBatch(item.batch_id)) return true;
    return false;
  }

  function uploadsBlocked(item) {
    if (allowDespiteHalt(item)) return false;
    if (haltState || (function () {
      try {
        return !!(global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__);
      } catch (e) {
        return false;
      }
    })()) {
      return true;
    }
    return isHaltedFor(item || {});
  }

  /** True when uploads go same-origin first-party (/g5) — lower handshake cost, no CORS preflight. */
  function isFirstPartyMode() {
    try {
      if (global.__GR_FIRST_PARTY__) return true;
      var b = global.__GR_BOOT__ || {};
      if (b.first_party || b.firstParty) return true;
      var raw = String(cfg.apiBase || b.apiBase || "").trim();
      if (raw.charAt(0) === "/") return true;
      if (raw && typeof location !== "undefined" && location.origin) {
        return new URL(raw, location.href).origin === location.origin;
      }
    } catch (eFp) {}
    return false;
  }

  /** Resolve relative first-party apiBase (/g5) against page origin. */
  function resolvedApiBase() {
    var raw = String(cfg.apiBase || global.__GR_BOOT__ && global.__GR_BOOT__.apiBase || "").trim();
    if (!raw && (global.__GR_FIRST_PARTY__ || (global.__GR_BOOT__ && global.__GR_BOOT__.first_party))) {
      raw = "/g5";
    }
    if (!raw) return "";
    if (/^https?:\/\//i.test(raw)) return raw.replace(/\/$/, "");
    if (raw.indexOf("//") === 0) {
      try {
        return (location.protocol + raw).replace(/\/$/, "");
      } catch (e0) {
        return ("https:" + raw).replace(/\/$/, "");
      }
    }
    if (raw.charAt(0) !== "/") raw = "/" + raw;
    raw = raw.replace(/\/$/, "");
    try {
      return (location.origin + raw).replace(/\/$/, "");
    } catch (e1) {
      return raw;
    }
  }

  /**
   * iss/70 P2: first-party may raise *caps*, never force high floor via max-stack.
   * effectiveUploadCap() is the final gate.
   */
  function applyFirstPartyPerfHints() {
    if (!isFirstPartyMode()) return;
    try {
      var mc = Number(global.__GR_UPLOAD_CONCURRENCY__ || 0);
      var mr = Number(global.__GR_MID_RAMP_CONCURRENCY__ || 0);
      var man =
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null;
      var bh = (man && man.brain_hints) || {};
      var maxC = Number(cfg.concurrency_max) || 6;
      // Requests may raise concurrency_max, not stamp concurrency to 12/16.
      if (mc > 0) cfg.concurrency_max = Math.min(8, Math.max(maxC, Math.min(mc, 8)));
      if (mr > 0) cfg.mid_ramp_concurrency = Math.min(cfg.concurrency_max, Math.max(cfg.mid_ramp_concurrency || 0, Math.min(mr, 6)));
      if (bh.upload_concurrency != null) {
        var req = Number(bh.upload_concurrency) || 0;
        if (req > 0) cfg.concurrency_max = Math.min(8, Math.max(cfg.concurrency_max || 6, Math.min(req, 8)));
      }
      if (bh.mid_ramp_concurrency != null) {
        var reqM = Number(bh.mid_ramp_concurrency) || 0;
        if (reqM > 0) {
          cfg.mid_ramp_concurrency = Math.min(
            cfg.concurrency_max || 6,
            Math.max(cfg.mid_ramp_concurrency || 0, Math.min(reqM, 6))
          );
        }
      }
      // Keep base concurrency at floor..max, never inflate to 12+.
      cfg.concurrency = Math.min(
        cfg.concurrency_max || 6,
        Math.max(cfg.concurrency_floor || 1, Number(cfg.concurrency) || 3)
      );
    } catch (eMan) {}
  }

  function postOne(item) {
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var base = resolvedApiBase() || String(cfg.apiBase || "").replace(/\/$/, "");
    var url = base + "/v1/ingest";
    var productVer =
      (global.__GR_PRODUCT_VERSION__ ||
        (global.__GR_BOOT__ &&
          (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
        "") + "";
    // iss/72: freeze once at capture boundary; never mutate material after freeze.
    freezeCapture(item);
    var payload = item.payload || {};
    // Integrity: frozen wire material must match capture; restore if collector aliased.
    try {
      if (item._capture_frozen && item._frozen_payload_json) {
        var nowS = stableStringify((payload && payload.fields) || {});
        if (nowS !== item._frozen_payload_json) {
          var restored = false;
          try {
            var snap =
              item._frozen_fields && typeof item._frozen_fields === "object"
                ? deepCloneJson(item._frozen_fields)
                : null;
            if (snap && typeof snap === "object") {
              if (!item.payload || typeof item.payload !== "object") item.payload = {};
              item.payload.fields = snap;
              payload = item.payload;
              item.payload_hash = payloadHashOf(payload);
              item.material_hash = item.payload_hash;
              restored = stableStringify(snap) === item._frozen_payload_json;
            }
          } catch (eRest) {
            restored = false;
          }
          try {
            if (global.GROps && GROps.report) {
              // Restored shared-ref churn → info-level noise control; true unrestorable → warn.
              GROps.report(
                restored ? "capture_mutation_restored" : "capture_mutated_blocked",
                "upload",
                { batch_id: item.batch_id, restored: !!restored },
                restored ? "info" : "warn"
              );
            }
          } catch (eM) {}
        }
      }
    } catch (ePv) {}
    var planEpoch = null;
    try {
      if (item.plan_epoch != null) planEpoch = Number(item.plan_epoch);
      else if (global.__GR_PLAN_EPOCH__ != null) planEpoch = Number(global.__GR_PLAN_EPOCH__);
      else if (global.GRSessionScheduler && GRSessionScheduler.getPlanEpoch) {
        planEpoch = Number(GRSessionScheduler.getPlanEpoch());
      }
      if (!isFinite(planEpoch) || planEpoch <= 0) planEpoch = null;
    } catch (ePe) {
      planEpoch = null;
    }
    var body = {
      session_id: item.session_id || cfg.session_id || global.__GR_SESSION_ID__ || "",
      batch_id: item.batch_id,
      source: item.source || "main",
      analyze: false,
      inject_path: item.inject_path || cfg.inject_path,
      product_version: productVer || undefined,
      payload: payload,
      capture_id: item.capture_id || undefined,
      material_generation: item.material_generation != null ? item.material_generation : undefined,
      payload_hash: item.payload_hash || undefined,
      material_hash: item.material_hash || item.payload_hash || undefined,
      // iss/72: stamp plan_epoch so server can reject stale route material.
      plan_epoch: planEpoch != null ? planEpoch : undefined,
      // attempt_id is transport-only — changes every send, not material identity.
      attempt_id:
        "att_" +
        String(item._transport_attempts || 0) +
        "_" +
        String(Date.now()),
      source_kind: "fe",
      realm_kind: (function () {
        try {
          var src = String(item.source || "main").toLowerCase();
          if (src.indexOf("worker") >= 0) return "worker";
          if (src.indexOf("sandbox") >= 0) return "sandbox_iframe";
          if (src.indexOf("iframe") >= 0) return "iframe";
          return "document";
        } catch (eRk) {
          return "document";
        }
      })(),
      probe_method_id:
        (item.probe_method_id ||
          item.method_id ||
          (item.method && item.method.method_id) ||
          undefined) || undefined,
    };
    // Never beacon after halt — pagehide would re-flood completed cycles.
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var sameOrigin = false;
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    var ctrl = null;
    try {
      if (typeof AbortController !== "undefined") {
        ctrl = new AbortController();
        try {
          ctrl.__gr_batch_id = String((item && item.batch_id) || "");
        } catch (eTag) {}
        inflightCtrls.push(ctrl);
      }
    } catch (eC) {}
    var fp = sameOrigin || isFirstPartyMode();
    // Gecko/Edge: hard anchors must prefer first-party same-origin (CORS dual-fire 5xx).
    var hard = isHardAnchorBatch(item.batch_id);
    var engineGecko = false;
    var engineEdge = false;
    try {
      var uaE = String((global.navigator && navigator.userAgent) || "");
      // No InstallTrigger (deprecated — typeof alone warns in Firefox).
      try {
        engineGecko =
          typeof global.mozInnerScreenX === "number" ||
          /Firefox\//.test(uaE) ||
          /FxiOS\//.test(uaE) ||
          (/Gecko\//.test(uaE) && !/like Gecko/.test(uaE));
      } catch (eG) {
        engineGecko = /Firefox\//.test(uaE);
      }
      engineEdge = /Edg\//.test(uaE);
    } catch (eEg) {}
    // Gecko/Edge: always first-party same-origin ingest (CORS dual-fire → NetworkError / mid incomplete).
    var forceFpEngine = engineGecko || engineEdge;
    if (forceFpEngine && !sameOrigin) {
      try {
        var p = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
        url = p + "/v1/ingest";
        sameOrigin = true;
        fp = true;
      } catch (eFp) {}
    }

    // Session seal: async prepare (grant from open). Beacon cannot await crypto — skip when sealing.
    // CRITICAL under REQUIRE_SEALED: never plain-fallback (server 426 storm). Lab-only plain when not required.
    function sealNeedNow() {
      try {
        if (global.__GR_FORCE_PLAIN_INGEST__) return false;
        if (global.__GR_SEEN_SEALED_REQUIRED__ || global.__GR_REQUIRE_SEALED__) return true;
        if (global.GRSeal && typeof GRSeal.isRequireSealed === "function" && GRSeal.isRequireSealed())
          return true;
        var b = global.__GR_BOOT__ || {};
        if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) return true;
        // Prod first-party: assume sealed before open/bootstrap lands (kill plain→426 race).
        if (
          (global.__GR_FIRST_PARTY__ || b.first_party || b.firstParty) &&
          (b.inject_path === "nginx" ||
            b.injectPath === "nginx" ||
            (b.env_id && String(b.env_id).indexOf("prod") === 0))
        ) {
          return true;
        }
      } catch (eN) {}
      return false;
    }
    /** Wait for open.seal_grant (or timeout) when sealed required — no plain attempt. */
    function waitOpenSealGrant(timeoutMs) {
      timeoutMs = timeoutMs == null ? 16000 : timeoutMs;
      try {
        if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
          return Promise.resolve(true);
        }
        if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
          if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
          return Promise.resolve(true);
        }
      } catch (e0) {}
      if (global.__GR_SEAL_READY_P__) {
        return Promise.race([
          global.__GR_SEAL_READY_P__.then(function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            return !!(
              (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) ||
              (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64)
            );
          }),
          new Promise(function (resolve) {
            setTimeout(function () {
              resolve(false);
            }, timeoutMs);
          }),
        ]);
      }
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          try {
            if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
              return resolve(true);
            }
            if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
              if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
              return resolve(true);
            }
          } catch (e1) {}
          if (Date.now() - start > timeoutMs) return resolve(false);
          setTimeout(tick, 35);
        }
        tick();
      });
    }

    function ensureSealModule() {
      if (global.GRSeal && typeof GRSeal.prepareUpload === "function") {
        return Promise.resolve(true);
      }
      if (!sealNeedNow()) return Promise.resolve(false);
      if (global.__GR_SEAL_LOAD_P__) return global.__GR_SEAL_LOAD_P__;
      global.__GR_SEAL_LOAD_P__ = new Promise(function (resolve) {
        try {
          var basePath = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
          var ver =
            (global.__GR_SERVER_PRODUCT_VERSION__ ||
              global.__GR_PRODUCT_VERSION__ ||
              (global.__GR_BOOT__ && (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
              "") + "";
          var src =
            basePath +
            "/dist/gr.seal.min.js" +
            (ver ? "?v=" + encodeURIComponent(ver) : "");
          var s = document.createElement("script");
          s.async = true;
          s.src = src;
          s.onload = function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            resolve(!!(global.GRSeal && GRSeal.prepareUpload));
          };
          s.onerror = function () {
            resolve(false);
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (eL) {
          resolve(false);
        }
      });
      return global.__GR_SEAL_LOAD_P__;
    }
    // Seal circuit: stop seal storms without re-collect thrash (GPI Phase C).
  function sealCircuitOpen() {
    try {
      if (global.__GR_SEAL_WASM_DEAD__) return true;
      var c = global.__GR_SEAL_CIRCUIT__;
      if (c && c.open && c.until && Date.now() < c.until) return true;
      if (c && c.open && c.until && Date.now() >= c.until) {
        global.__GR_SEAL_CIRCUIT__ = { open: false, fails: 0, until: 0 };
      }
    } catch (e) {}
    return false;
  }
  function sealCircuitTrip(why) {
    try {
      var c = global.__GR_SEAL_CIRCUIT__ || { fails: 0 };
      c.fails = (c.fails || 0) + 1;
      if (c.fails >= 3) {
        c.open = true;
        c.until = Date.now() + 30000;
        c.why = String(why || "seal_fail");
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_circuit_open",
              "upload",
              { fails: c.fails, until: c.until, why: c.why },
              "warn"
            );
          }
        } catch (eO) {}
      }
      global.__GR_SEAL_CIRCUIT__ = c;
    } catch (e2) {}
  }

  var sealPrep = ensureSealModule()
      .then(function () {
        if (sealCircuitOpen()) {
          var ec = new Error("seal_circuit_open");
          ec.code = "seal_circuit_open";
          ec.seal_failed = true;
          ec.seal_circuit = true;
          throw ec;
        }
        // When sealed required: never race plain — wait open grant first (hard batches especially).
        if (!sealNeedNow()) return true;
        var waitMs = hard || isDeepenBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 20000 : 14000;
        return waitOpenSealGrant(waitMs).then(function (ok) {
          if (ok) return true;
          // Open lag / wiped grant: remint via session open once, then short wait.
          return refreshSealGrant(sessionOf(item)).then(function (got) {
            if (!got) return false;
            return waitOpenSealGrant(4000);
          });
        });
      })
      .then(function () {
      if (!(global.GRSeal && typeof GRSeal.prepareUpload === "function")) {
        if (sealNeedNow()) {
          var eMiss = new Error("seal_module_missing");
          eMiss.code = "seal_failed";
          eMiss.seal_failed = true;
          throw eMiss;
        }
        return {
          url: url,
          body: JSON.stringify(body),
          sealed: false,
        };
      }
      try {
        if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eAd0) {}
      // Prefer authoritative session id after open supersede.
      try {
        var authSid =
          (global.__GR_SESSION_ID__ || cfg.session_id || body.session_id || "") + "";
        if (authSid && body.session_id && authSid !== body.session_id) {
          // Only rewrite when auth looks like a real cycle and grant matches it.
          if (
            GRSeal.grantValid &&
            GRSeal.grantValid(authSid) &&
            String(authSid).indexOf("cycle_") === 0
          ) {
            body.session_id = authSid;
            item.session_id = authSid;
          }
        }
      } catch (eSid) {}
      return GRSeal.prepareUpload(base, body).catch(function (eSeal) {
        var need = sealNeedNow();
        if (need) {
          var es = eSeal || new Error("seal_failed");
          es.code = es.code || "seal_failed";
          es.seal_failed = true;
          // Permanent WASM death (unsupported / not binary / load fail): stop seal retry storm.
          var msgSeal = String(es.message || es || "");
          var wasmDead =
            !!global.__GR_SEAL_WASM_DEAD__ ||
            /seal_wasm_unsupported|wasm_not_binary|wasm_http_|seal_wasm_required/i.test(msgSeal);
          if (wasmDead) {
            try {
              global.__GR_SEAL_WASM_DEAD__ = 1;
              es.code = /unsupported/i.test(msgSeal) ? "seal_wasm_unsupported" : "seal_failed";
              es.terminal = false; // still allow remint if grant path heals; budget handled below
              es.seal_wasm_dead = true;
            } catch (eDead) {}
          }
          try {
            // Cap ops: crawlers emitted 60–70× upload_5xx/session on seal_wasm_required.
            global.__GR_SEAL_OPS_N__ = (global.__GR_SEAL_OPS_N__ || 0) + 1;
            if (global.__GR_SEAL_OPS_N__ <= 3 && global.GROps && GROps.uploadHttp) {
              GROps.uploadHttp(0, item && item.batch_id, {
                seal_failed: true,
                seal_wasm_dead: !!wasmDead,
                code: es.code,
                err: msgSeal.slice(0, 160),
                final: !!wasmDead && global.__GR_SEAL_OPS_N__ >= 2,
              });
            }
          } catch (eOpsS) {}
          throw es;
        }
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_fallback_plain",
              "upload",
              {
                batch: String((item && item.batch_id) || "").slice(0, 48),
                err: String((eSeal && eSeal.message) || eSeal || "").slice(0, 80),
              },
              "warn"
            );
          }
        } catch (eRep) {}
        return {
          url: base + "/v1/ingest",
          body: JSON.stringify(body),
          sealed: false,
        };
      });
    });

    return sealPrep.then(function (wire) {
    url = wire.url || url;
    var bodyStr = wire.body || JSON.stringify(body);
    var sealed = !!wire.sealed;
    // Keep item session aligned with sealed envelope (supersede path).
    try {
      if (wire.session_id && item) item.session_id = wire.session_id;
    } catch (eWs) {}
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo2) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    fp = sameOrigin || isFirstPartyMode();

    // Sealed path: never sendBeacon (async crypto; envelope size). Use fetch+keepalive on pagehide.
    if (hiding && !sealed && global.navigator && navigator.sendBeacon) {
      try {
        var blob = new Blob([bodyStr], { type: "text/plain;charset=UTF-8" });
        if (navigator.sendBeacon(url, blob)) return Promise.resolve({ ok: true, via: "beacon" });
      } catch (e) {}
    }
    // Gecko: avoid keepalive on large hard plain bodies (broken-pipe 5xx).
    // Sealed+hide: always keepalive so unload does not drop encrypted batch (no sendBeacon).
    var useKeepalive = false;
    if (sealed && hiding) {
      useKeepalive = true;
    } else if (hiding || (fp && hard && !engineGecko && !engineEdge)) {
      useKeepalive = true;
    }
    var init = {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: bodyStr,
      // Same-origin: send CF cookies; cross-origin: include when CORS credentials allowed
      credentials: sameOrigin ? "same-origin" : "include",
      mode: sameOrigin ? "same-origin" : "cors",
      keepalive: !!useKeepalive,
    };
    if (ctrl) init.signal = ctrl.signal;
    // Abort hard uploads that hang so SLA can retry.
    // Gecko: hard multipath payloads are large — short abort caused timeout storms
    // (upload_timeout_abort → probe_fail_budget → mid blocked).
    var abortTo = null;
    if (ctrl && (hard || isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id))) {
      try {
        // Heavy deepen needs longer: large sealed body + compress.
        var hardAbortMs = isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id)
          ? engineGecko || engineEdge
            ? 45000
            : 36000
          : engineGecko || engineEdge
            ? 28000
            : 22000;
        abortTo = setTimeout(function () {
          try {
            if (ctrl) ctrl.__gr_timeout_abort = 1;
            ctrl.abort();
          } catch (eAb) {}
        }, hardAbortMs);
      } catch (eT) {}
    }
    // First / high-priority batches: high fetch priority (v57 p50 path)
    try {
      if (sent === 0 || (item.priority || 0) >= 90 || hard) {
        init.priority = "high";
      } else if (fp) {
        // Remaining packs: still elevated so relay hop is not starved by page assets
        init.priority = "high";
      }
    } catch (eP) {}
    return fetch(url, init)
      .finally(function () {
        if (abortTo) {
          try {
            clearTimeout(abortTo);
          } catch (eC2) {}
        }
      })
      .then(function (r) {
        // Drop controller from list when response arrives
        if (ctrl) {
          var ix = inflightCtrls.indexOf(ctrl);
          if (ix >= 0) inflightCtrls.splice(ix, 1);
        }
        // If we halted while request was in flight, do not parse/retry as error flood.
        if (uploadsBlocked(item) && r.status === 410) {
          applyHalt("session_expired", sessionOf(item), "cycle_complete");
          var errH = new Error("upload_halted");
          errH.terminal = true;
          errH.status = 410;
          errH.silent = true;
          errH.code = "cycle_complete";
          throw errH;
        }
        return r
          .json()
          .catch(function () {
            // HTTP 2xx with empty/non-JSON body is still a successful transport —
            // do NOT invent ok:false (was flooding upload_biz_reject with "HTTP 200").
            if (r.ok) {
              return { ok: true, empty_body: true, http: r.status };
            }
            return { ok: false, error: "HTTP " + r.status };
          })
          .then(function (j) {
            var term = parseTerminal(r.status, j || {});
            if (term.terminal) {
              // Halt BEFORE throwing so sibling inflight sees blocked flag.
              applyHalt(term.code || "session_expired", sessionOf(item), term.code);
              // Soft 200 close: resolve quietly (no throw → no red XHR / NS_BINDING_ABORTED cascade).
              if (term.soft_close || (r.status === 200 && term.terminal)) {
                try {
                  if (global.GROps && GROps.report) {
                    GROps.report(
                      "upload_soft_cycle_closed",
                      "upload",
                      {
                        code: term.code,
                        batch: String((item && item.batch_id) || "").slice(0, 48),
                      },
                      "info"
                    );
                  }
                } catch (eSoft) {}
                return j || { ok: true, accepted: false, halt_uploads: true };
              }
              var err = new Error((j && j.error) || "HTTP " + r.status);
              err.terminal = true;
              err.status = r.status;
              err.code = term.code;
              err.body = j;
              err.silent = r.status === 410 || r.status === 200;
              throw err;
            }
            // Transport OK: never treat as biz reject (even if body omitted ok).
            if (r.ok) {
              if (j && j.ok === false && j.error) {
                // Explicit server business reject on 200 — report warn, still accept storage if ingest landed.
                try {
                  if (global.GROps && GROps.uploadHttp) {
                    GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                      ok: false,
                      err: j.error,
                      explicit_biz: true,
                    });
                  }
                } catch (eOpsU0) {}
              }
              // Prior network fail for this batch → recovery event (vtid-tagged).
              try {
                var bidOk = String((item && (item.batch_id || item.pack_id)) || "");
                global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
                var prevFails = global.__GR_NET_FAIL_N__[bidOk] || 0;
                if (prevFails > 0 && global.GROps && GROps.uploadRecovered) {
                  GROps.uploadRecovered(bidOk, {
                    prior_network_fails: prevFails,
                    http: r.status,
                    sealed: !!sealed,
                  });
                  delete global.__GR_NET_FAIL_N__[bidOk];
                }
              } catch (eRec) {}
              return j || { ok: true };
            }
            // HTTP 403/503: sniff CF "I'm Under Attack" / challenge HTML.
            if (r.status === 403 || r.status === 503 || r.status === 429) {
              try {
                var snip =
                  (j && (j.error || j.message || JSON.stringify(j).slice(0, 120))) ||
                  "HTTP " + r.status;
                var snipL = String(snip).toLowerCase();
                var cfHit =
                  /challenge|cf-mitigated|just a moment|attention required|under attack|cloudflare|cf-ray|access denied/i.test(
                    snipL
                  );
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                    ok: false,
                    err: String(snip).slice(0, 100),
                    body_snip: String(snip).slice(0, 100),
                    net_class: cfHit ? "edge_challenge" : "http_" + r.status,
                    cf_challenge: !!cfHit,
                    will_retry: true,
                  });
                }
                var eCf = new Error(String(snip).slice(0, 120));
                eCf.status = r.status;
                eCf.body = j;
                eCf.network = !!cfHit; // retriable when edge challenge
                eCf.code = cfHit ? "upload_edge_challenge" : "upload_http_" + r.status;
                eCf.cf_challenge = !!cfHit;
                throw eCf;
              } catch (eCfThrow) {
                if (eCfThrow && eCfThrow.status) throw eCfThrow;
              }
            }
            if (j && j.ok === false) {
              try {
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(
                    r.status,
                    item && (item.batch_id || item.pack_id),
                    { ok: j && j.ok, err: j && j.error }
                  );
                }
              } catch (eOpsU) {}
              var e2 = new Error((j && j.error) || "HTTP " + r.status);
              e2.status = r.status;
              e2.body = j;
              // Broken pipe / client abort often surfaces as TypeError network fail upstream.
              e2.network = false;
              // 426 sealed_ingest_required: flip require flag, never retry plain.
              try {
                var errTxt = String((j && j.error) || "");
                if (
                  r.status === 426 ||
                  /sealed_ingest_required/i.test(errTxt)
                ) {
                  global.__GR_SEEN_SEALED_REQUIRED__ = true;
                  global.__GR_REQUIRE_SEALED__ = true;
                  global.__GR_BOOT__ = global.__GR_BOOT__ || {};
                  global.__GR_BOOT__.require_sealed_ingest = true;
                  if (global.GRSeal && GRSeal.setRequireSealed) {
                    GRSeal.setRequireSealed(true);
                  }
                  e2.code = "sealed_required";
                  e2.seal_required = true;
                  e2.seal_failed = true; // retriable path (clear sentKeys for hard)
                }
              } catch (e426) {}
              throw e2;
            }
            // Non-JSON error body with 426
            if (r.status === 426) {
              try {
                global.__GR_SEEN_SEALED_REQUIRED__ = true;
                global.__GR_REQUIRE_SEALED__ = true;
                if (global.GRSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
              } catch (e426b) {}
              var e426e = new Error("sealed_ingest_required");
              e426e.status = 426;
              e426e.code = "sealed_required";
              e426e.seal_required = true;
              e426e.seal_failed = true;
              throw e426e;
            }
            return j;
          });
      })
      .catch(function (e) {
        if (ctrl) {
          var iy = inflightCtrls.indexOf(ctrl);
          if (iy >= 0) inflightCtrls.splice(iy, 1);
        }
        // Aborted due to halt / true unload / timeout — silent (never flood ops as network).
        // Tab background (PAGE_BACKGROUNDED) is NOT terminal — keep retrying for maximize probe.
        var msgE = String((e && (e.message || e.name)) || e || "");
        var unloading = isPageUnloading();
        if (
          e &&
          (e.name === "AbortError" ||
            /abort/i.test(msgE) ||
            (haltState && !e.status) ||
            unloading ||
            global.__GR_HALT_UPLOADS__)
        ) {
          var ea = new Error("upload_halted");
          ea.terminal = !!haltState || unloading;
          ea.status = haltState ? 410 : 0;
          ea.silent = true;
          ea.code = (haltState && haltState.code) || (unloading ? "pagehide" : "halt");
          // Soft timeout abort (hard hang): allow retry, not terminal cycle close.
          if (
            !haltState &&
            !unloading &&
            (/abort/i.test(msgE) || (ctrl && ctrl.__gr_timeout_abort))
          ) {
            ea.terminal = false;
            ea.silent = true;
            ea.network = false; // do not flood upload_network
            ea.status = 0;
            ea.code = "upload_timeout_abort";
            ea.timeout_abort = true;
          }
          throw ea;
        }
        // Network / broken-pipe: report once per batch (not every retry).
        // Classify CF challenge / abort / background so ops is not pure "error flood".
        if (e && !e.status) {
          try {
            var bidN = String((item && (item.batch_id || item.pack_id)) || "");
            var hideN = unloading;
            var netKey = "net:" + bidN;
            global.__GR_NET_REPORT__ = global.__GR_NET_REPORT__ || Object.create(null);
            global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
            var lastN = global.__GR_NET_REPORT__[netKey] || 0;
            var nowN = Date.now();
            var msgLow = String(msgE || "").toLowerCase();
            var netClass = "fetch_fail";
            if (e.timeout_abort || /abort/i.test(msgLow)) netClass = "abort";
            else if (
              /challenge|cf-mitigated|just a moment|attention required|access denied|under attack|cf-ray|cloudflare/i.test(
                msgLow
              )
            )
              netClass = "edge_challenge";
            else if (isPageBackgrounded()) netClass = "backgrounded";
            // Do not count silent/timeout aborts as network flood.
            if (e.silent || e.timeout_abort || netClass === "abort") {
              e.network = false;
              e.status = 0;
              e.code = e.code || "upload_timeout_abort";
            } else {
              global.__GR_NET_FAIL_N__[bidN] = (global.__GR_NET_FAIL_N__[bidN] || 0) + 1;
              var attN = global.__GR_NET_FAIL_N__[bidN];
              var maxAtt = maxAttemptsForItem(item);
              var willRetryN = attN < maxAtt && !hideN;
              if (
                !hideN &&
                nowN - lastN > 12000 &&
                global.GROps &&
                GROps.uploadHttp
              ) {
                global.__GR_NET_REPORT__[netKey] = nowN;
                // Sub-classify http:0 Failed to fetch (not server HTTP status).
                // online=false → offline; else client_fetch_fail (conn limit / DNS / TLS / CORS).
                var online =
                  typeof navigator !== "undefined" && navigator.onLine != null
                    ? !!navigator.onLine
                    : null;
                if (online === false) netClass = "offline";
                else if (netClass === "fetch_fail") netClass = "client_fetch_fail";
                var apiBaseN = "";
                var apiHostN = "";
                try {
                  apiBaseN = String(
                    cfg.apiBase ||
                      (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) ||
                      global.__GR_API_BASE__ ||
                      "/g5"
                  );
                  apiHostN =
                    apiBaseN.indexOf("http") === 0
                      ? apiBaseN.split("/").slice(0, 3).join("/")
                      : location && location.host
                        ? String(location.protocol) + "//" + location.host + apiBaseN
                        : apiBaseN;
                } catch (eAb) {}
                // Remember last net_class per batch for exhaust reports.
                try {
                  global.__GR_NET_CLASS_LAST__ =
                    global.__GR_NET_CLASS_LAST__ || Object.create(null);
                  global.__GR_NET_CLASS_LAST__[bidN] = {
                    net_class: netClass,
                    transport: "no_http_response",
                    ts: nowN,
                  };
                } catch (eMem) {}
                GROps.uploadHttp(0, bidN, {
                  network: true,
                  err: msgE.slice(0, 80),
                  net_class: netClass,
                  // Not an HTTP response from API — browser never got status line.
                  transport: "no_http_response",
                  fail_class_hint:
                    netClass === "edge_challenge"
                      ? "edge_challenge"
                      : netClass === "offline"
                        ? "client_offline"
                        : netClass === "abort" || netClass === "backgrounded"
                          ? "client_abort"
                          : "network_env",
                  impact_band_hint: isHeavyBatch(bidN) ? "deepen_optional" : "ops_attention",
                  api_base: apiBaseN.slice(0, 80),
                  api_host: String(apiHostN).slice(0, 120),
                  online: online,
                  backgrounded: isPageBackgrounded(),
                  unloading: !!hideN || isPageUnloading(),
                  pagehide: !!hideN,
                  short_visit: !!hideN || isPageUnloading(),
                  commercial_hard: isCommercialHardForOps(bidN),
                  commercial_land_fast: isCommercialLandFast(bidN),
                  attempt: attN,
                  max_attempts: maxAtt,
                  will_retry: willRetryN,
                  final: !willRetryN,
                  heavy: isHeavyBatch(bidN),
                });
              }
              e.network = true;
              e.status = 0;
              e.code = "upload_network";
            }
          } catch (eNet) {}
        }
        throw e;
      });
    });
  }

  function maxAttempts() {
    return hiding ? cfg.max_attempts_hide : cfg.max_attempts_alive;
  }

  function isRpaBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isRpaBatch) {
        return !!GRProbeLifecycle.isRpaBatch(batchId);
      }
    } catch (eR) {}
    var id = String(batchId || "");
    return id === "B11_interaction" || id.indexOf("B11_") === 0;
  }

  function maxAttemptsForItem(item) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.maxAttemptsFor) {
        var cap = GRProbeLifecycle.maxAttemptsFor(item && item.batch_id);
        if (hiding) return Math.min(cap, cfg.max_attempts_hide || 4);
        // RPA continuous: hard cap 3 alive, 2 on hide — never storm soft_exhausted.
        if (isRpaBatch(item && item.batch_id)) {
          return Math.min(cap, hiding ? 2 : 3);
        }
        return cap;
      }
    } catch (eM) {}
    if (isRpaBatch(item && item.batch_id)) return hiding ? 2 : 3;
    return maxAttempts();
  }

  /**
   * B10 + Lane-C must-land: beat short-visit / pagehide window.
   * Must NOT inherit deepen/heavy 4s+ stretch (lifecycle marks these heavy).
   */
  function isCommercialLandFast(batchId) {
    var id = String(batchId || "");
    return (
      id === "B10_hw_curves" ||
      id === "mid.curves" ||
      id === "B10x_silicon_noderiv" ||
      id === "B10x_silicon_rint" ||
      id === "B10x_silicon_ulp"
    );
  }

  function backoffMsForItem(item, attempt) {
    var a = Math.max(0, (attempt || 1) - 1);
    // Commercial land path: front-load retries inside ~2–4s dwell + keepalive.
    if (isCommercialLandFast(item && item.batch_id)) {
      // 120, 360, 680, 1080, 1560… cap 2000
      return Math.min(2000, 120 + a * 240 + a * a * 40);
    }
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.backoffMsFor) {
        return GRProbeLifecycle.backoffMsFor(item && item.batch_id, attempt);
      }
    } catch (eB) {}
    return Math.min(20000, 1000 * Math.pow(2, a));
  }

  function scheduleRetry(item, attempt) {
    var delay = backoffMsForItem(item, attempt);
    // Heavy network fails: longer backoff so browser connection budget recovers.
    // Never stretch commercial land — short visits lose B10/Lane-C otherwise.
    try {
      var bid = item && item.batch_id;
      if (!isCommercialLandFast(bid) && isHeavyBatch(bid)) {
        delay = Math.max(delay, 4000 + Math.min(20000, (attempt || 1) * 3000));
      }
    } catch (eHb) {}
    // iss/70: transport-only retry — keep frozen capture; do not re-collect.
    try {
      freezeCapture(item);
      transportRetryCount++;
      item._transport_attempts = (item._transport_attempts || 0) + 1;
    } catch (eTr) {}
    setTimeout(function () {
      if (haltState && !allowDespiteHalt(item)) return;
      if (uploadsBlocked(item)) return;
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          return;
        }
      } catch (eA) {}
      // Coalesce force retries (same key) — avoid pending storms after transport fail.
      queuePending(item);
      pump();
    }, delay);
  }

  function pump() {
    try {
      pruneStaleInflight();
    } catch (ePr) {}
    var conc = effectiveUploadCap();
    if (conc <= 0) return;
    // After halt, still pump B10x / force_after_halt (EDH must-land).
    if (
      haltState &&
      !pending.some(function (p) {
        return allowDespiteHalt(p);
      })
    ) {
      return;
    }
    // Never start more than conc slots even if counter drifted.
    if (inflight > conc) inflight = Math.min(inflight, inflightItems.length, conc);
    while (inflight < conc && pending.length) {
      pending.sort(function (a, b) {
        return effectivePriority(b) - effectivePriority(a);
      });
      var item = pending.shift();
      if (uploadsBlocked(item)) {
        haltedDrops++;
        continue;
      }
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          // Soft quiet budget: drop soft packs, keep hard.
          if (!isHardAnchorBatch(item.batch_id)) {
            failed++;
            continue;
          }
        }
      } catch (eQ) {}
      // Deferred retry: not ready yet.
      if (item._retry_after_ms && item._retry_after_ms > Date.now()) {
        queuePending(item);
        break;
      }
      // P1: serialize heavy (B10x/B7) — max 1 inflight to cut Failed to fetch storms.
      var itemHeavy = isHeavyBatch(item.batch_id);
      if (itemHeavy && heavyInflight >= heavyMaxInflight()) {
        // Prefer a light pack next; requeue this heavy for later.
        var lightIdx = -1;
        for (var hi = 0; hi < pending.length; hi++) {
          if (!isHeavyBatch(pending[hi].batch_id)) {
            lightIdx = hi;
            break;
          }
        }
        if (lightIdx >= 0) {
          var light = pending.splice(lightIdx, 1)[0];
          queuePending(item);
          item = light;
          itemHeavy = false;
        } else {
          pending.unshift(item);
          break;
        }
      }
      var k = keyOf(item);
      attempts[k] = (attempts[k] || 0) + 1;
      if (hiding && isHardAnchorBatch(item.batch_id) && attempts[k] > 1) {
        hardRetries++;
      }
      inflight++;
      inflightItems.push(item);
      registryUpsertFromItem(item, "uploading");
      if (itemHeavy) heavyInflight++;
      // iss P0: bind each in-flight upload to its own item/k/heavy — never close over
      // function-scoped `var` from the pump while-loop (concurrent ACK mis-attribution).
      startUpload(item, itemHeavy, k);
    }
  }

  function startUpload(item, itemHeavy, k) {
      postOne(item)
        .then(function (resp) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          if (haltState) {
            pump();
            return;
          }
          sent++;
          if (isHardAnchorBatch(item.batch_id)) hardFlushed++;
          // iss/72 P0-4: only verified ACK → sentKeys / batches_ok.
          try {
            var ackRes = applyAck(item, resp || {});
            var ackT = ackRes && typeof ackRes === "object" ? ackRes.type : ackRes;
            var verified = ackRes && typeof ackRes === "object" ? !!ackRes.verified : false;
            if (ackT === "conflict") {
              try {
                if (global.GROps && GROps.report) {
                  GROps.report(
                    "upload_ack_conflict",
                    "upload",
                    {
                      batch_id: item.batch_id,
                      generation: item.material_generation,
                      payload_hash: item.payload_hash,
                    },
                    "warn"
                  );
                }
              } catch (eCf) {}
            } else if (verified) {
              sentKeys[k] = true;
              try {
                sentKeys[k + "|ts"] = Date.now();
              } catch (eTs) {}
              if (resp && resp.sealed === false) noteOutcome("plain_ok", item.batch_id, item);
              else noteOutcome("sealed_ok", item.batch_id, item);
              registryUpsertFromItem(item, "acked");
            } else {
              // transport_ok_unverified — do not short-circuit future retries incorrectly
              registryUpsertFromItem(item, "transport_ok_unverified");
            }
          } catch (eOut) {}
          try {
            aimdOnSuccess();
          } catch (eAim) {}
          // Mild ramp after first success — never exceed concurrency_max.
          if (!ramped && cfg.ramp_after_first) {
            ramped = true;
            var maxC = Number(cfg.concurrency_max) || 6;
            var rampT = Math.min(maxC, Number(cfg.ramp_after_first) || 4);
            if (cfg.concurrency < rampT) cfg.concurrency = rampT;
          }
          // Success-path terminal signals (complete / cool) without waiting for 410.
          var termOk = parseTerminal(200, resp || {});
          if (termOk.terminal) {
            applyHalt(termOk.code || "probe_complete", sessionOf(item), termOk.code);
          } else {
            maybeLocalIdentityDone();
          }
          try {
            // Keep client-side received inventory for hard-SLA (no cache-clear required).
            global.__GR_RECEIVED_BATCHES__ = global.__GR_RECEIVED_BATCHES__ || [];
            global.__GR_RECEIVED_BATCHES__.push({
              batch_id: item.batch_id,
              source: item.source || "main",
            });
            global.dispatchEvent(
              new CustomEvent("gr-upload-ok", {
                detail: {
                  batch_id: item.batch_id,
                  source: item.source || "main",
                  response: resp || null,
                  cycle_probe_status: (resp && resp.cycle_probe_status) || null,
                },
              })
            );
          } catch (eEv) {}
          pump();
        })
        .catch(function (err) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          try {
            if (err && (err.seal_required || err.status === 426)) noteOutcome("sealed_reject", item.batch_id);
            else if (err && err.network) noteOutcome("network", item.batch_id);
            else if (err && !err.silent) noteOutcome("other_fail", item.batch_id);
          } catch (eOut2) {}
          try {
            if (!err || !err.silent) aimdOnFail();
          } catch (eAimF) {}
          try {
            registryUpsertFromItem(item, "retry_wait");
          } catch (eReg) {}
          // Terminal: cycle done / cool / session_expired — never retry flood.
          if (err && err.terminal) {
            applyHalt(
              err.code || "session_expired",
              sessionOf(item),
              err.code || "session_expired"
            );
            if (!err.silent) failed++;
            else haltedDrops++;
            try {
              if (!err.silent) {
                global.dispatchEvent(
                  new CustomEvent("gr-upload-terminal", {
                    detail: {
                      batch_id: item.batch_id,
                      status: err.status,
                      code: err.code,
                      error: String(err.message || err),
                    },
                  })
                );
              }
            } catch (eT) {}
            // Do not pump after terminal — queue drained in applyHalt.
            return;
          }
          if (uploadsBlocked(item) || haltState) {
            haltedDrops++;
            return;
          }
          try {
            if (!err || !err.silent) {
              global.dispatchEvent(
                new CustomEvent("gr-upload-fail", {
                  detail: {
                    batch_id: item.batch_id,
                    pack_id: item.pack_id || item.batch_id,
                    status: err && err.status,
                    code: err && err.code,
                    network: !!(err && err.network),
                    error: String((err && err.message) || err || ""),
                  },
                })
              );
            }
          } catch (eFailEv) {}
          // Hard / deepen / seal fail: clear sentKeys so re-upload can land (never stick "sent").
          try {
            var bidR = String(item.batch_id || "");
            var retriableFail =
              (err && err.network) ||
              (err && err.cf_challenge) ||
              (err &&
                (err.code === "seal_failed" ||
                  err.code === "sealed_required" ||
                  err.seal_failed ||
                  err.seal_required)) ||
              (err && err.timeout_abort) ||
              (err && err.status === 426) ||
              // Edge 403/503 (CF under-attack / rate) — retry hard/deepen anchors.
              (err &&
                (err.status === 403 || err.status === 503 || err.status === 429) &&
                (isHardAnchorBatch(bidR) || isDeepenBatch(bidR)));
            if (retriableFail) {
              // Always clear dedupe on seal/426 so sealed retry can land any batch.
              if (
                err.seal_failed ||
                err.seal_required ||
                err.status === 426 ||
                isHardAnchorBatch(bidR) ||
                isDeepenBatch(bidR) ||
                bidR === "B27_storage_privacy" ||
                bidR === "B65_client_hints_full" ||
                bidR === "B18_webgpu" ||
                bidR === "B46_audio_deep" ||
                bidR === "B55_webrtc_ice_deep" ||
                bidR === "B2_hardware" ||
                bidR === "B3_system" ||
                bidR === "B0_bootstrap" ||
                bidR === "B1_conflict" ||
                bidR === "B11_interaction" ||
                bidR === "B12_anti_camouflage" ||
                bidR === "B7_sandbox"
              ) {
                delete sentKeys[k];
              }
            }
          } catch (eSk) {}
          // seal_failed / 426 is never cycle-terminal — always retry path above.
          var isSealRace =
            err &&
            (err.code === "seal_failed" ||
              err.code === "sealed_required" ||
              err.seal_failed ||
              err.seal_required ||
              err.status === 426 ||
              /seal_grant|seal_timeout|seal_module/i.test(String((err && err.message) || "")));
          if (isSealRace) {
            try {
              err.terminal = false;
            } catch (eTerm) {}
            try {
              sealCircuitTrip(err && (err.code || err.message));
            } catch (eCt) {}
            // Permanent WASM failure: limited seal waits then stop (no 70× storm).
            try {
              var wasmDeadQ =
                !!(err && err.seal_wasm_dead) ||
                !!global.__GR_SEAL_WASM_DEAD__ ||
                /seal_wasm_unsupported|wasm_not_binary|seal_wasm_required/i.test(
                  String((err && err.message) || "")
                );
              item._seal_waits = (item._seal_waits || 0) + 1;
              if (wasmDeadQ && item._seal_waits >= 2) {
                err.terminal = true;
                err.code = err.code || "seal_wasm_dead";
                // Do not decrement attempt — count as real fail.
              } else if ((attempts[k] || 0) > 0) {
                // Do not burn hard-attempt budget on open-lag seal races.
                attempts[k] = Math.max(0, attempts[k] - 1);
              }
              // Circuit open: defer retries longer (transport-only, no re-collect).
              if (sealCircuitOpen()) {
                item._retry_after_ms = Date.now() + 8000;
              }
            } catch (eDec) {}
            // Align session with grant before retry (open supersede).
            try {
              var fixSid =
                (global.__GR_SESSION_ID__ ||
                  (global.GRSeal &&
                    global.GRSeal.resolveSealSessionId &&
                    GRSeal.resolveSealSessionId(item.session_id)) ||
                  cfg.session_id ||
                  "") + "";
              if (fixSid && String(fixSid).indexOf("cycle_") === 0) {
                item.session_id = fixSid;
              }
            } catch (eFix) {}
            // Remint grant in parallel (coalesced).
            try {
              refreshSealGrant(sessionOf(item));
            } catch (eRf) {}
          }
          var cap = maxAttemptsForItem(item);
          var nTry = attempts[k] || 1;
          var sealWaits = item._seal_waits || 0;
          // Seal race: large seal_wait budget; real attempts still capped separately.
          if (isSealRace) {
            cap = Math.max(cap, isHardAnchorBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 14 : 10);
            // Allow continue while seal waits < 16 even if attempt counter looks high.
            if (sealWaits < 16 && nTry >= cap) {
              nTry = cap - 1;
              attempts[k] = nTry;
            }
          }
          if (nTry < cap || (isSealRace && sealWaits < 16)) {
            if (isHardAnchorBatch(item.batch_id) && !isSealRace) hardRetries++;
            if ((item.priority || 0) >= 70) {
              item.priority = Math.min(100, (item.priority || 70) + 1);
            }
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail && !isSealRace) {
                GRProbeLifecycle.recordFail(
                  item.batch_id,
                  (err && (err.code || (err.timeout_abort && "upload_timeout_abort") || err.message)) ||
                    "upload_fail"
                );
              }
            } catch (eLf) {}
            // Seal grant race: wait for remint / open grant (not exponential storm).
            if (isSealRace) {
              var sealDelay = Math.min(4000, 300 + sealWaits * 400);
              item._retry_after_ms = Date.now() + sealDelay;
              queuePending(item);
              setTimeout(function () {
                try {
                  if (item._retry_after_ms && item._retry_after_ms <= Date.now()) {
                    delete item._retry_after_ms;
                  }
                } catch (eClr) {}
                pump();
              }, sealDelay + 20);
              pump();
            } else {
              // Delayed requeue — avoid spin retry storms.
              scheduleRetry(item, nTry);
              pump();
            }
          } else {
            failed++;
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail) {
                var fr = GRProbeLifecycle.recordFail(item.batch_id, "attempts_exhausted");
                // Dedupe exhausted reports per batch (was flooding ops).
                var exKey = "ex:" + String(item.batch_id || "");
                global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                var lastEx = global.__GR_EXHAUST_REPORT__[exKey] || 0;
                var nowEx = Date.now();
                if (nowEx - lastEx > 30000 && global.GROps && GROps.report) {
                  global.__GR_EXHAUST_REPORT__[exKey] = nowEx;
                  // Commercial hard only → error; B10x/B47 network exhaust → warn (v150 fix).
                  var commercialHard = isCommercialHardForOps(item.batch_id);
                  var deepenish =
                    isDeepenBatch(item.batch_id) ||
                    isHeavyBatch(item.batch_id) && !commercialHard;
                  var exCode = commercialHard
                    ? "upload_hard_exhausted"
                    : deepenish
                      ? "upload_deepen_exhausted"
                      : "upload_soft_exhausted";
                  var exSev = commercialHard ? "error" : "warn";
                  // Partial success: B10 already landed + only deepen failed → always warn.
                  try {
                    if (
                      !commercialHard &&
                      sessionOutcome.batches_ok &&
                      sessionOutcome.batches_ok["B10_hw_curves"]
                    ) {
                      exSev = "warn";
                    }
                  } catch (ePart) {}
                  var apiHost = "";
                  try {
                    var ab = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5");
                    apiHost = ab.indexOf("http") === 0 ? ab.split("/").slice(0, 3).join("/") : ab;
                  } catch (eH) {}
                  // Ops dimensions: attach last net_class for this batch (if any).
                  var lastNc = null;
                  var lastTransport = null;
                  try {
                    var netHist = global.__GR_NET_CLASS_LAST__ || Object.create(null);
                    var nk = String(item.batch_id || "");
                    if (netHist[nk]) {
                      lastNc = netHist[nk].net_class || null;
                      lastTransport = netHist[nk].transport || null;
                    }
                  } catch (eNc) {}
                  GROps.report(
                    exCode,
                    "upload",
                    {
                      batch_id: item.batch_id,
                      attempts: nTry,
                      over_budget: !!(fr && fr.overBudget),
                      commercial_hard: commercialHard,
                      deepen: !!deepenish,
                      b10_already_ok: !!(
                        sessionOutcome.batches_ok &&
                        sessionOutcome.batches_ok["B10_hw_curves"]
                      ),
                      api_base: apiHost,
                      online:
                        typeof navigator !== "undefined" && navigator.onLine != null
                          ? !!navigator.onLine
                          : null,
                      // taxonomy dimensions (server also normalizes)
                      net_class: lastNc || undefined,
                      transport: lastTransport || undefined,
                      fail_class_hint: commercialHard
                        ? "hard_exhaust"
                        : deepenish
                          ? "deepen_exhaust"
                          : "soft_exhaust",
                      impact_band_hint: commercialHard
                        ? "commercial_critical"
                        : "deepen_optional",
                    },
                    exSev
                  );
                }
              }
            } catch (eEx) {}
            pump();
          }
        });
  }

  var api = {
    configure: function (o) {
      o = o || {};
      var prevSid = cfg.session_id;
      Object.keys(o).forEach(function (k) {
        if (o[k] !== undefined) cfg[k] = o[k];
      });
      // New session id → isolate queue/registry (iss/72 P1-2).
      if (o.session_id && String(o.session_id) !== String(prevSid || "")) {
        if (haltState && haltState.session_id !== String(o.session_id)) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
          } catch (eC) {}
        }
        resetSessionState("configure_session_id");
      }
      // First-party /g5: raise caps only within min/max budget.
      applyFirstPartyPerfHints();
    },
    /** Mark keys already retained server-side (from open/session received list). */
    seedSentKeys: function (keys) {
      (keys || []).forEach(function (k) {
        if (typeof k === "string") sentKeys[k] = true;
        else if (k && k.batch_id) {
          sentKeys[keyOf(k)] = true;
        }
      });
    },
    /**
     * Clear dedupe keys so force_recollect can re-upload the same batch
     * (e.g. B2 present without residual → brain requests recollect).
     */
    clearSentKeys: function (items) {
      (items || []).forEach(function (it) {
        if (typeof it === "string") {
          delete sentKeys[it];
        } else if (it && it.batch_id) {
          var bidClr = String(it.batch_id);
          // Budget: after terminal ok, only one force_recollect clear per session.
          if (sessionOutcome.batches_ok[bidClr] && forceRecollectUsed[bidClr]) {
            return;
          }
          if (sessionOutcome.batches_ok[bidClr]) {
            forceRecollectUsed[bidClr] = 1;
          }
          delete sentKeys[keyOf(it)];
          try {
            delete sessionOutcome.batches_ok[bidClr];
            delete sessionOutcome.batches_started[bidClr];
          } catch (eOk) {}
        }
      });
    },
    /**
     * Whether brain force_recollect may clear + re-upload this batch.
     * Terminal ok batches: at most once per session.
     */
    allowForceRecollect: function (batchId) {
      var bid = String(batchId || "");
      if (!bid) return false;
      if (!sessionOutcome.batches_ok[bid]) return true;
      if (forceRecollectUsed[bid]) return false;
      forceRecollectUsed[bid] = 1;
      return true;
    },
    enqueue: function (item) {
      if (!item || !item.batch_id) return;
      var bidEnq = String(item.batch_id || "");
      var recollect =
        item.force_recollect === true ||
        item.forceRecollect === true ||
        item.reset_attempts === true;
      // New material generation on explicit recollect.
      if (recollect) {
        try {
          item._capture_frozen = false;
          item.material_generation = (item.material_generation || 1) + 1;
          item.capture_id = null;
        } catch (eRc) {}
      }
      // Success short-circuit: already sealed_ok → skip unless brain force_recollect.
      // (Was: B10x/secondary always force=true → 5–10× wire storms, iss/65.)
      if (batchAlreadyOk(bidEnq) && !recollect) {
        // Explicit force without recollect still blocked once ok (start heartbeat spam).
        successShortCircuit++;
        skipped++;
        return;
      }
      // pagehide: reject non-allowlisted packs immediately.
      if ((hiding || isPageUnloading()) && !isPagehideAllowlisted(bidEnq) && !recollect) {
        pagehideDropped++;
        skipped++;
        return;
      }
      // B10x EDH: force only until first sealed_ok (or recollect). Survive halt/cool.
      if (isB10xBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Secondary silicon/infra: same — once landed, no force storm.
      if (isSecondaryInfraBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Hard/deepen attempt ceiling across SLA re-kicks (prevents attempts:16 storms).
      try {
        var kCap = keyOf(item);
        var capEnq = maxAttemptsForItem(item);
        // B10 / B10x / seal remint: higher ceiling + grant-ready reset (short-visit residual).
        if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if (item.reset_attempts) {
          delete attempts[kCap];
          delete item._seal_waits;
        }
        // If grant is now valid after seal race, clear fake exhaustion and continue.
        try {
          if (
            (attempts[kCap] || 0) >= capEnq &&
            global.GRSeal &&
            GRSeal.grantRawValid &&
            GRSeal.grantRawValid()
          ) {
            attempts[kCap] = Math.max(0, capEnq - 4);
            item._seal_waits = 0;
          }
        } catch (eGr) {}
        // Secondary infra always gets elevated attempt cap
        if (isSecondaryInfraBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if ((attempts[kCap] || 0) >= capEnq && !item.reset_attempts) {
          // Soft re-arm hard/B10x every 8s instead of permanent drop.
          if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id) || isSecondaryInfraBatch(item.batch_id)) {
            var lastCap = item._cap_block_ms || 0;
            var rearmMs = isHardAnchorBatch(item.batch_id) && !isB10xBatch(item.batch_id) ? 6000 : 8000;
            if (Date.now() - lastCap > rearmMs) {
              item._cap_block_ms = Date.now();
              attempts[kCap] = Math.max(0, capEnq - 5);
              item._seal_waits = 0;
              try {
                refreshSealGrant(sessionOf(item));
              } catch (eR2) {}
            } else {
              skipped++;
              try {
                if (global.GROps && GROps.report) {
                  var exK = "excap:" + String(item.batch_id || "");
                  global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                  var nowC = Date.now();
                  if (nowC - (global.__GR_EXHAUST_REPORT__[exK] || 0) > 60000) {
                    global.__GR_EXHAUST_REPORT__[exK] = nowC;
                    GROps.report(
                      "upload_hard_exhausted",
                      "upload",
                      {
                        batch_id: item.batch_id,
                        attempts: attempts[kCap],
                        reason: "enqueue_cap",
                        rearm_ms: rearmMs,
                      },
                      "warn"
                    );
                  }
                }
              } catch (eCap) {}
              return;
            }
          } else {
            skipped++;
            return;
          }
        }
      } catch (eEnqCap) {}
      // Hard gate: after 410/complete never enqueue (ignore force — force only skips dedupe),
      // except B10x / force_after_halt.
      if (uploadsBlocked(item) && !allowDespiteHalt(item)) {
        haltedDrops++;
        skipped++;
        return;
      }
      try {
        if (
          (global.__GR_STOP_PROBE__ || global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) &&
          !allowDespiteHalt(item)
        ) {
          haltedDrops++;
          skipped++;
          return;
        }
      } catch (eStop) {}
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          if (!isHardAnchorBatch(item.batch_id)) {
            skipped++;
            return;
          }
        }
      } catch (eAe) {}
      var k = keyOf(item);
      // RPA continuous: never force-bypass queue coalescing (gecko flood).
      var rpa = isRpaBatch(item.batch_id);
      // Terminal success short-circuit (not start heartbeat). force alone is not enough.
      if (batchAlreadyOk(bidEnq) && !recollect) {
        successShortCircuit++;
        skipped++;
        return;
      }
      // sentKeys: allow one upgrade from start→done (started marked sentKeys but not batches_ok).
      if (sentKeys[k] && !item.force && !recollect && batchAlreadyOk(bidEnq)) {
        skipped++;
        return;
      }
      if (sentKeys[k] && !item.force && !recollect && !isStartHeartbeatItem(item)) {
        // Final payload after start: need force or clear. midEnqueue sets force when !alreadySent;
        // after start alreadySent is true — allow if only batches_started (not batches_ok).
        if (!sessionOutcome.batches_started[bidEnq] || sessionOutcome.batches_ok[bidEnq]) {
          skipped++;
          return;
        }
        // Upgrade path: start was sent, final multipath ready — allow once.
        item.force = true;
      }
      // For RPA, even force: coalesce; never on unload (pagehide drops B11).
      if (rpa && (hiding || isPageUnloading())) {
        pagehideDropped++;
        skipped++;
        return;
      }
      if (rpa && sentKeys[k] && !item.pagehide_flush && !recollect) {
        // Allow re-upload only after quiet window (30s) — was 5s and still noisy.
        var lastOk = sentKeys[k + "|ts"] || 0;
        if (Date.now() - lastOk < 30000) {
          skipped++;
          return;
        }
      }
      // Freeze only after all dedupe, lifecycle, pagehide, and terminal gates.
      // Self-heal may replay many descriptors after cycle completion; dropped
      // items must not pay the deep-clone/hash cost or inflate capture metrics.
      try {
        if (item.payload) freezeCapture(item);
      } catch (eFzEnq) {}
      // Dedup pending by session|batch|source — including force (dense hedge / secondary).
      // Previous: force=true skipped replace → same B19 could stack dozens of times.
      queuePending(item);
      pump();
    },
    /**
     * When open returns a different cycle id than the client mint (completed sticky
     * bag superseded), retarget pending items so they land on the live cycle.
     */
    rewriteSessionId: function (fromId, toId) {
      var from = String(fromId || "");
      var to = String(toId || "");
      if (!from || !to || from === to) return 0;
      var n = 0;
      for (var i = 0; i < pending.length; i++) {
        var it = pending[i];
        if (!it) continue;
        var sid = String(it.session_id || cfg.session_id || "");
        if (sid === from || !it.session_id) {
          it.session_id = to;
          n++;
        }
      }
      try {
        cfg.session_id = to;
      } catch (eC) {}
      // Drop sentKeys bound to the dead cycle so re-upload of same batch is allowed.
      try {
        Object.keys(sentKeys).forEach(function (k) {
          if (String(k).indexOf(from + "|") === 0) delete sentKeys[k];
        });
      } catch (eK) {}
      // If we halted only because of 410 on the dead cycle, allow the live cycle to proceed.
      if (haltState && String(haltState.session_id || "") === from) {
        haltState = null;
        try {
          global.__GR_HALT_UPLOADS__ = false;
          global.__GR_STOP_PROBE__ = false;
          global.__GR_SKIP_IDENTITY__ = false;
          global.__GR_CYCLE_CLOSED__ = null;
          global.__GR_UPLOAD_HALT__ = null;
          global.__GR_PHASE__ = "active";
          if (!cfg.concurrency || cfg.concurrency < 1) cfg.concurrency = 3;
          // Undo false local cool stamped by 410 on the superseded bag.
          var S = global.GRStorage;
          if (S && S.setCoolUntil) S.setCoolUntil(0);
        } catch (eR) {}
        pump();
      }
      // Open supersede often arrives with seal_grant for `to` — wake seal waiters.
      pump();
      return n;
    },
    /** Wake pump after open.seal_grant / hot-swap re-eval (B0 may be grant-waiting). */
    kick: function () {
      try {
        if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eK) {}
      // Clear stale seal retry gates so grant-ready items run immediately.
      try {
        for (var i = 0; i < pending.length; i++) {
          if (pending[i] && pending[i]._retry_after_ms) {
            delete pending[i]._retry_after_ms;
          }
        }
      } catch (eR) {}
      pump();
    },
    /** Stop identity uploads for a cycle (cool open / 410 complete). */
    halt: function (reason, sessionId, code) {
      applyHalt(reason || "halt", sessionId, code);
      return haltState;
    },
    isHalted: function (sessionId) {
      if (!haltState) return false;
      if (sessionId == null || sessionId === "") return true;
      return !haltState.session_id || haltState.session_id === String(sessionId);
    },
    haltState: function () {
      return haltState ? Object.assign({}, haltState) : null;
    },
    /** Snapshot of FE local view for multi-party reconcile with BE. */
    clientView: function () {
      var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
      var sent = [];
      try {
        Object.keys(sentKeys).forEach(function (k) {
          var parts = String(k).split("|");
          if (parts.length >= 2) sent.push(parts[1]);
        });
      } catch (eS) {}
      return {
        session_id: sid,
        stop_probe: !!global.__GR_STOP_PROBE__,
        skip_identity: !!global.__GR_SKIP_IDENTITY__,
        halted: !!haltState,
        phase: global.__GR_PHASE__ || null,
        local_uploads_done: !!(
          global.__GR_IDENTITY_UPLOADS_DONE__ &&
          global.__GR_IDENTITY_UPLOADS_DONE__.session_id === sid
        ),
        sent_batch_ids: sent,
        last_http_status: (haltState && haltState.status) || null,
        last_error_code: (haltState && haltState.code) || null,
      };
    },
    /**
     * Apply BE reconcile corrections (server authoritative for cycle lifecycle).
     * Returns applied action names.
     */
    applyCorrections: function (reconcileBody) {
      var applied = [];
      if (!reconcileBody) return applied;
      var sid =
        (reconcileBody.session_id ||
          cfg.session_id ||
          global.__GR_SESSION_ID__ ||
          "") + "";
      try {
        if (reconcileBody.cycle_probe_status) {
          global.__GR_CYCLE_PROBE_STATUS__ = reconcileBody.cycle_probe_status;
        }
        if (reconcileBody.business_state) {
          global.__GR_BUSINESS_STATE__ = reconcileBody.business_state;
        }
      } catch (eG) {}
      var list = reconcileBody.corrections || [];
      for (var i = 0; i < list.length; i++) {
        var c = list[i] || {};
        var act = c.action || "";
        if (act === "fe_halt_uploads" || reconcileBody.fe_should_halt) {
          // Refuse cool-halt while primary residual not acked and still in flight/pending.
          // Server may claim identity_complete_cool from B10x-only has_b10 false positive.
          var refuseHalt = false;
          try {
            var hasPrimary =
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) ||
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]);
            var hardPending = false;
            var pi;
            for (pi = 0; pi < pending.length; pi++) {
              if (isHardAnchorBatch(pending[pi] && pending[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            for (pi = 0; !hardPending && pi < inflightItems.length; pi++) {
              if (isHardAnchorBatch(inflightItems[pi] && inflightItems[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            var reasonStr = String((c && c.reason) || reconcileBody.business_state || "");
            if (
              !hasPrimary &&
              (hardPending || reasonStr.indexOf("identity_complete") >= 0 || reasonStr.indexOf("cycle_status=complete") >= 0)
            ) {
              refuseHalt = true;
            }
          } catch (eRef) {}
          if (refuseHalt) {
            applied.push("fe_halt_uploads_refused_missing_b10");
          } else {
            applyHalt(c.reason || "reconcile_halt", sid, c.reason || "reconcile");
            applied.push("fe_halt_uploads");
          }
        } else if (
          act === "fe_resume_if_same_active_cycle" ||
          act === "fe_clear_false_cool" ||
          reconcileBody.fe_should_resume
        ) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
            global.__GR_CYCLE_CLOSED__ = null;
            global.__GR_STOP_PROBE__ = false;
            global.__GR_SKIP_IDENTITY__ = false;
            global.__GR_PHASE__ = "active";
          } catch (eR) {}
          applied.push(act || "fe_resume");
        } else if (act === "fe_mark_local_done_only") {
          maybeLocalIdentityDone();
          applied.push(act);
        } else if (act === "rebind_session_id") {
          applied.push(act);
        } else if (act === "be_completed_cycle" || act === "be_complete_cycle") {
          applyHalt("be_complete", sid, "cycle_complete");
          applied.push(act);
        }
      }
      if (reconcileBody.fe_should_halt && !haltState) {
        applyHalt("reconcile_halt", sid, reconcileBody.business_state || "halt");
        applied.push("fe_should_halt");
      }
      if (reconcileBody.aligned && reconcileBody.halt_uploads && !haltState) {
        applyHalt("aligned_halt", sid, reconcileBody.business_state || "halt");
        applied.push("aligned_halt");
      }
      try {
        global.dispatchEvent(
          new CustomEvent("gr-status-reconciled", {
            detail: {
              applied: applied,
              body: reconcileBody,
              session_id: sid,
            },
          })
        );
      } catch (eE) {}
      return applied;
    },
    /** New page cycle: clear halt so a fresh session_id can upload. */
    resume: function (newSessionId) {
      if (newSessionId && haltState && haltState.session_id === String(newSessionId)) {
        // Same id still closed.
        return haltState;
      }
      haltState = null;
      // Restore pump capacity for a new cycle.
      if (!cfg.concurrency || cfg.concurrency < 1) {
        cfg.concurrency = 3;
        ramped = false;
      }
      try {
        global.__GR_UPLOAD_HALT__ = null;
        global.__GR_CYCLE_CLOSED__ = null;
        global.__GR_HALT_UPLOADS__ = false;
        // Do not clear STOP_PROBE here — boot decides for cool windows.
      } catch (eR) {}
      return null;
    },
    /** B0 enqueued: allow parallel rest while first upload still inflight. */
    softRamp: function (n) {
      var maxC = Number(cfg.concurrency_max) || 6;
      var target = Math.min(maxC, Number(n) || cfg.mid_ramp_concurrency || 4);
      if (!ramped && cfg.concurrency < target) {
        cfg.concurrency = target;
      }
      pump();
    },
    /**
     * Flush pending uploads.
     * - pagehide / beforeunload / unload → true unload (PAGE_HIDING, lower hide attempts)
     * - hidden / background → tab not focused; keep alive retries; do NOT stop probe
     */
    flush: function (reason) {
      flushReason = reason || "flush";
      var r = String(reason || "");
      var unloading =
        r === "pagehide" || r === "beforeunload" || r === "unload" || r === "close";
      var background = r === "hidden" || r === "background" || r === "visibility_hidden";
      if (unloading) {
        hiding = true;
        try {
          global.__GR_PAGE_HIDING__ = true;
          global.__GR_PAGE_UNLOADING__ = true;
        } catch (e) {}
        // Abort non-allowlisted inflight so keepalive slots go to hard/B10x (iss/65).
        try {
          var keptCtrls = [];
          for (var ai = 0; ai < inflightCtrls.length; ai++) {
            var c = inflightCtrls[ai];
            var cBid = c && c.__gr_batch_id ? String(c.__gr_batch_id) : "";
            if (cBid && isPagehideAllowlisted(cBid)) {
              keptCtrls.push(c);
              continue;
            }
            try {
              if (c && typeof c.abort === "function") {
                c.__gr_soft_unload_abort = 1;
                c.abort();
                softAbortOnUnload++;
              }
            } catch (eAb) {}
          }
          inflightCtrls = keptCtrls;
        } catch (eAc) {}
        // Drop pending non-allowlisted (mid/R/B11) — boost alone is not enough.
        try {
          var kept = [];
          for (var pi = 0; pi < pending.length; pi++) {
            var pit = pending[pi];
            if (!pit) continue;
            if (isPagehideAllowlisted(pit.batch_id) && !batchAlreadyOk(pit.batch_id)) {
              kept.push(pit);
            } else {
              pagehideDropped++;
            }
          }
          pending = kept;
        } catch (eDrop) {}
        // Keepalive window is tiny — max concurrency for remaining hard/B10x only.
        try {
          var maxC = Number(cfg.concurrency_max) || 6;
          cfg.concurrency = Math.max(Number(cfg.concurrency) || 3, Math.min(maxC, 6));
        } catch (eConc) {}
      } else if (background) {
        // iss/70: background lowers budget; do not maximize.
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
          global.__GR_PAGE_HIDING__ = false;
          // Cap concurrency while backgrounded.
          cfg.concurrency = Math.min(Number(cfg.concurrency) || 3, 2);
        } catch (eBg) {}
      }
      // Boost hard anchors still pending so flush prioritizes commercial materials.
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id) || isDeepenBatch(pending[i].batch_id)) {
          pending[i].priority =
            (pending[i].priority || 0) +
            (isHardAnchorBatch(pending[i].batch_id)
              ? cfg.hard_anchor_priority_boost || 1000
              : 200);
          pending[i].hard_anchor_flush = isHardAnchorBatch(pending[i].batch_id);
        }
      }
      pump();
      return api.stats();
    },
    /** Tab visible again — restore full concurrency and clear background flag. */
    markVisible: function () {
      hiding = false;
      try {
        global.__GR_PAGE_BACKGROUNDED__ = false;
        // Never clear true unload flags here (page is going away).
        if (!global.__GR_PAGE_UNLOADING__) {
          global.__GR_PAGE_HIDING__ = false;
        }
      } catch (eV) {}
      applyFirstPartyPerfHints();
      if (!ramped && cfg.concurrency < (cfg.mid_ramp_concurrency || 8)) {
        cfg.concurrency = Math.max(cfg.concurrency, cfg.mid_ramp_concurrency || 8);
      }
      pump();
    },
    markHiding: function () {
      // Legacy: treat as background unless already unloading.
      if (!isPageUnloading()) {
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
        } catch (eM) {}
      } else {
        hiding = true;
      }
    },
    isHardAnchorBatch: isHardAnchorBatch,
    isDeepenBatch: isDeepenBatch,
    isCommercialLandFast: isCommercialLandFast,
    backoffMsForItem: backoffMsForItem,
    isPageUnloading: isPageUnloading,
    isPageBackgrounded: isPageBackgrounded,
    /** Pure: commercial/hard final complete (exported for boot multi-tick). */
    hardFinalComplete: hardFinalComplete,
    parseTerminal: parseTerminal,
    alreadySent: function (item) {
      return !!sentKeys[keyOf(item || {})];
    },
    /** iss/70 P0: batch has frozen capture pending upload/retry (hard-SLA must not re-collect). */
    hasPendingCapture: hasPendingCapture,
    /** iss/70: acked | conflict | uploading | retry_wait | queued | collected | absent */
    materialState: materialState,
    /**
     * Self-heal Loop T: re-pump pending or re-queue frozen capture for a batch.
     * Does not re-collect GPU materials.
     */
    nudgeTransport: function (batchId, sessionId) {
      var bid = String(batchId || "");
      if (!bid) {
        pump();
        return false;
      }
      var sidN = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
      var i;
      // Clear deferred retry so pump can take it now.
      for (i = 0; i < pending.length; i++) {
        if (String(pending[i].batch_id || "") !== bid) continue;
        if (sidN && pending[i].session_id && String(pending[i].session_id) !== sidN) continue;
        try {
          delete pending[i]._retry_after_ms;
        } catch (eClr) {}
      }
      pump();
      return hasPendingCapture(bid, sidN);
    },
    /** Force pump (self-heal / tests). */
    pump: pump,
    freezeCapture: freezeCapture,
    payloadHashOf: payloadHashOf,
    applyAck: applyAck,
    registryGet: registryGet,
    registrySnapshot: function () {
      var out = {};
      Object.keys(materialRegistry).forEach(function (k) {
        out[k] = materialRegistry[k];
      });
      return out;
    },
    effectiveUploadCap: effectiveUploadCap,
    stats: function () {
      var hardPending = 0;
      var pendingBatches = [];
      var seenB = {};
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id)) hardPending++;
        var pb = String((pending[i] && pending[i].batch_id) || "");
        if (pb && !seenB[pb]) {
          seenB[pb] = 1;
          pendingBatches.push(pb);
        }
      }
      return {
        pending: pending.length,
        pending_batches: pendingBatches.slice(0, 32),
        inflight: inflight,
        inflight_items: inflightItems.length,
        concurrency: cfg.concurrency,
        concurrency_max: cfg.concurrency_max,
        effective_cap: effectiveUploadCap(),
        sent: sent,
        failed: failed,
        skipped_dedupe: skipped,
        success_short_circuit: successShortCircuit,
        pagehide_dropped: pagehideDropped,
        soft_abort_on_unload: softAbortOnUnload,
        flush_reason: flushReason,
        hard_anchor_pending: hardPending,
        hard_anchor_flushed: hardFlushed,
        hard_anchor_retries: hardRetries,
        hard_anchor_priority_boost: cfg.hard_anchor_priority_boost || 1000,
        transport_retries: transportRetryCount,
        capture_freezes: captureFreezeCount,
        ack_stored: ackStored,
        ack_duplicate: ackDuplicate,
        ack_merged: ackMerged,
        ack_conflict: ackConflict,
        aimd_ups: aimdUps,
        aimd_downs: aimdDowns,
        registry_n: Object.keys(materialRegistry).length,
        hiding: hiding,
        halted: !!haltState,
        halt_reason: haltState && haltState.reason,
        halt_code: haltState && haltState.code,
        halt_session_id: haltState && haltState.session_id,
        halted_drops: haltedDrops,
      };
    },
    /** Pure helper for tests: sort order under hide with hard boost. */
    sortPreview: function (items, hide) {
      var was = hiding;
      if (hide) hiding = true;
      var copy = (items || []).slice().map(function (it) {
        return {
          batch_id: it.batch_id,
          priority: it.priority || 0,
          effective: effectivePriority(it),
          hard: isHardAnchorBatch(it.batch_id),
        };
      });
      copy.sort(function (a, b) {
        return b.effective - a.effective;
      });
      hiding = was;
      return copy;
    },
    /** Resolve when upload queue is idle (or timeout). Used by multi-tick settle. */
    whenIdle: function (timeoutMs) {
      timeoutMs = timeoutMs == null ? 2000 : timeoutMs;
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          if (pending.length === 0 && inflight === 0) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW) {}
            resolve(api.stats());
            return;
          }
          if (Date.now() - start >= timeoutMs) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW2) {}
            resolve(api.stats());
            return;
          }
          setTimeout(tick, 16);
        }
        tick();
      });
    },
    /** P2: probe depth class for current session outcome. */
    probeDepthClass: function () {
      try {
        var ok = sessionOutcome.batches_ok || {};
        var hasB0 = !!ok["B0_bootstrap"];
        var hasB10 = !!ok["B10_hw_curves"];
        var hasB8 = !!ok["B8_gateway"];
        var n = Object.keys(ok).length;
        if (hasB8 && !hasB0 && n <= 2) return "gateway_only";
        if (hasB0 && hasB10) return "browser_b0_b10";
        if (hasB0) return "browser_partial";
        if (n) return "browser_lite";
        return "empty";
      } catch (e) {
        return "empty";
      }
    },
    /**
     * Progressive land snapshot (window still open).
     * Used by self_heal / ops to drive missing_probe reduction without waiting for unload.
     */
    completenessSnapshot: function () {
      var ok = sessionOutcome.batches_ok || {};
      var hard = cfg.hard_anchor_batches || [];
      var hardOk = [];
      var hardMiss = [];
      for (var i = 0; i < hard.length; i++) {
        if (ok[hard[i]]) hardOk.push(hard[i]);
        else hardMiss.push(hard[i]);
      }
      var okKeys = Object.keys(ok);
      var ver = "";
      try {
        ver = String(
          global.__GR_PRODUCT_VERSION__ ||
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            global.__GR_FE_PACKS_VERSION__ ||
            ""
        );
      } catch (eV) {}
      return {
        progressive: true,
        product_version: ver,
        probe_depth_class: api.probeDepthClass(),
        hard_total: hard.length,
        hard_ok_n: hardOk.length,
        hard_ok: hardOk,
        hard_missing: hardMiss,
        batches_ok_n: okKeys.length,
        batches_ok: okKeys,
        pending: pending.length,
        inflight: inflight,
        halted: !!haltState,
        hiding: !!hiding,
        sealed_ok: sessionOutcome.sealed_ok || 0,
        network_fail: sessionOutcome.network || 0,
        main_complete: !!(ok["B0_bootstrap"] && ok["B10_hw_curves"]),
      };
    },
    /**
     * Keep draining queue while the tab is open (not only pagehide).
     * Safe no-op when empty / halted.
     */
    pumpWhileOpen: function (why) {
      if (haltState) return api.completenessSnapshot();
      try {
        if (typeof api.flush === "function") {
          api.flush(why || "progressive_open");
        }
      } catch (eP) {}
      try {
        pump();
      } catch (e2) {}
      return api.completenessSnapshot();
    },
  };

  global.GRUploadQueue = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- pack_loader.js ---- */
/**
 * First-pack sub-pack loader: priority kick + resource-class concurrency.
 *
 * Product v5.8.137 ResourceBus:
 * - Same resource class (gpu/audio/rtc/cpu/nest/verify): at most 1 global in-flight
 *   (main ∥ iframe ∥ worker share the bus).
 * - Different classes may run in parallel **only if packs do not share a class**.
 * - Compound packs (e.g. B10_hw_curves) multi-lock [gpu,audio,cpu] so secondary
 *   packs never thrash the same hardware during staged multipath.
 * - Light class races under maxLight cap.
 * - Per-source pack order may be shuffled (anti-spoof + stagger).
 * - Brain parallel_groups are class-aware stages; FE also enforces ResourceBus.
 * - Custom probes: GRPackLoader.registerCustomProbe({id, resourceClass|resourceClasses, run, ...}).
 */
(function (global) {
  "use strict";

  var health = Object.create(null);
  var kicked = Object.create(null);

  function setHealth(id, patch) {
    health[id] = Object.assign(health[id] || { pack_id: id }, patch, { ts: Date.now() });
    global.__GR_PACK_HEALTH__ = health;
  }

  /** Manifest (protocol 2) layers / asset_base from pin bootstrap. */
  function manifest() {
    try {
      return (
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null
      );
    } catch (e) {
      return null;
    }
  }

  function assetBase() {
    try {
      var m = manifest();
      if (m && m.asset_base) return String(m.asset_base).replace(/\/$/, "");
      if (global.__GR_ASSET_BASE__) return String(global.__GR_ASSET_BASE__).replace(/\/$/, "");
    } catch (eB) {}
    return "";
  }

  /**
   * True when basename is already Standard-C opaque or content-hashed:
   * - pure opaque: `a1b2c3d4e5f6.min.js`
   * - embedded: `gr.race.a1b2c3d4e5f6.min.js`
   */
  function isHashedOrOpaqueLeaf(name) {
    var base = String(name || "")
      .split("?")[0]
      .split("#")[0]
      .replace(/^.*\//, "");
    if (/^[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base)) return true;
    if (/\.[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base)) return true;
    return false;
  }

  /** Standard C: inject opaque gen into logical basename when content-hash missing. */
  function withGenFilename(logicalPath) {
    var p = String(logicalPath || "").replace(/^\//, "");
    var gen = "";
    try {
      gen = String(
        (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_gen) ||
          global.__GR_ASSET_GEN__ ||
          ""
      );
    } catch (eG) {}
    if (!gen || isHashedOrOpaqueLeaf(p)) return p;
    if (/\.min\.js$/i.test(p)) return p.replace(/\.min\.js$/i, "." + gen + ".min.js");
    if (/\.js$/i.test(p)) return p.replace(/\.js$/i, "." + gen + ".js");
    if (/\.wasm$/i.test(p)) return p.replace(/\.wasm$/i, "." + gen + ".wasm");
    if (/\.html$/i.test(p)) return p.replace(/\.html$/i, "." + gen + ".html");
    if (/\.css$/i.test(p)) return p.replace(/\.css$/i, "." + gen + ".css");
    return p;
  }

  /** Resolve layer URL from bootstrap manifest (hard/mid/dense/b10x/…). */
  function layerUrl(name) {
    try {
      var m = manifest();
      // Prefer full opaque/hashed URLs from bootstrap — do not rewrite.
      if (m && m.layers && m.layers[name] && typeof m.layers[name] === "string") {
        return String(m.layers[name]);
      }
      if (m && m.assets && m.assets[name]) return String(m.assets[name]);
      // Opaque pack tokens (logical stem → /dist/<hash>.min.js)
      var tokens =
        (m && m.layers && m.layers.pack_tokens) ||
        (m && m.pack_tokens) ||
        null;
      if (tokens && tokens[name]) return String(tokens[name]);
      // Dynamic packs: only via pack_tokens (opaque). Template {pack_token} is a
      // placeholder token already expanded in pack_tokens map — never invent
      // meaningful collector filenames on the wire.
      if (m && m.layers && m.layers.pack_url_template && name) {
        var tpl = String(m.layers.pack_url_template);
        // Reject templates that would emit logical pack ids (anti black-hat).
        if (tpl.indexOf("{pack_id}") >= 0) {
          return "";
        }
      }
    } catch (eL) {}
    // No meaningful-name fallback on the wire (opaque-only). Empty → caller skips.
    return "";
  }

  /**
   * Keep Standard-B content-hashed URLs intact.
   * Relative logical names join asset_base + gen-injected filename.
   */
  function withProductVer(src) {
    src = String(src || "");
    if (!src) return src;
    // Standard C: already opaque/hashed basename — collapse legacy version path only.
    if (isHashedOrOpaqueLeaf(src)) {
      return src.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    }
    // Collapse legacy /dist/v/<ver>/[g/<gen>/] → /dist/ (anti-leak).
    if (/\/dist\/v\/[^/]+\//.test(src)) {
      src = src.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    }
    // Prefer absolute asset_base from manifest when src is relative.
    try {
      var base = assetBase();
      if (base && src.charAt(0) !== "/" && src.indexOf("://") < 0) {
        base = String(base).replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
        return (
          String(base).replace(/\/$/, "") +
          "/" +
          withGenFilename(src.replace(/^\.\//, ""))
        );
      }
    } catch (eRel) {}
    // Flat or absolute: inject gen into basename only (never add version path).
    try {
      var m2 = src.match(/^(.*\/)([^/?#]+)([?#].*)?$/);
      if (m2 && !isHashedOrOpaqueLeaf(m2[2])) {
        return m2[1] + withGenFilename(m2[2]) + (m2[3] || "");
      }
    } catch (eInj) {}
    return src;
  }

  function loadScript(src, packId) {
    src = withProductVer(src);
    setHealth(packId, { load: "loading", src: src });
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = src;
      s.async = true;
      s.onload = function () {
        setHealth(packId, { load: "ok" });
        resolve();
      };
      s.onerror = function () {
        setHealth(packId, { load: "fail" });
        reject(new Error("load_failed " + src));
      };
      (document.head || document.documentElement).appendChild(s);
    });
  }

  /**
   * @param {Array<{id, priority?, src?, run?, batch_id?}>} packs
   * @param {object} ctx
   */
  function probeHalted() {
    try {
      if (global.__GR_STOP_PROBE__ || global.__GR_SKIP_IDENTITY__) return true;
      var Q = global.GRUploadQueue;
      if (Q && typeof Q.isHalted === "function" && Q.isHalted()) return true;
    } catch (e) {}
    return false;
  }

  /** Packs that must still run after identity halt (silicon/infra deepen). */
  function allowKickDespiteHalt(packOrId) {
    var id = typeof packOrId === "string" ? packOrId : String((packOrId && (packOrId.id || packOrId.pack_id || packOrId.batch_id)) || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return true;
    if (id === "B47_sab_clock" || id === "B18_webgpu" || id === "B46_audio_deep") return true;
    if (id === "B10_hw_curves") return true;
    return false;
  }

  /** True when primary residual B10 has not been upload-acked yet. */
  function primaryB10StillPending() {
    try {
      if (global.GRUploadQueue && GRUploadQueue.registryGet) {
        var st = GRUploadQueue.registryGet("B10_hw_curves", "main");
        if (st && st.state === "acked") return false;
        var st2 = GRUploadQueue.registryGet("mid.curves", "main");
        if (st2 && st2.state === "acked") return false;
      }
    } catch (eR) {}
    try {
      var so = global.__GR_SESSION_OUTCOME__ || global.__GR_LAST_RESULT__ || null;
      if (so && so.batches_ok) {
        if (so.batches_ok["B10_hw_curves"] || so.batches_ok["mid.curves"]) return false;
      }
    } catch (eS) {}
    try {
      // Server already saw primary B10
      var cps = global.__GR_CYCLE_PROBE_STATUS__ || {};
      if (cps.has_b10 === true) return false;
      var cov = cps.identity_coverage || {};
      if (cov.has_b10 === true) return false;
    } catch (eC) {}
    return true;
  }

  /**
   * While primary B10 not acked, exclusive packs that are NOT B10 must wait.
   * Prevents concurrent kickAll waves from starving multi-lock B10 forever.
   */
  function mustDeferForPrimaryB10(packOrId) {
    var id =
      typeof packOrId === "string"
        ? packOrId
        : String((packOrId && (packOrId.id || packOrId.pack_id || packOrId.batch_id)) || "");
    if (!id) return false;
    if (id === "B10_hw_curves" || id === "mid.curves") return false;
    // Light / nest / gateway-adjacent may proceed; exclusive silicon lanes wait for B10.
    var classes = resourceClasses(id);
    var onlyLight = classes.length === 1 && classes[0] === "light";
    if (onlyLight) return false;
    // Bootstrap / identity surface only — keep lean so B10 gets GPU soon.
    // (FingerprintJS/Thumbmark: collect critical entropy first, soft components later.)
    if (
      id === "B7_sandbox" ||
      id === "B0_bootstrap" ||
      id === "B1_conflict" ||
      id === "B11_interaction" ||
      id === "B8_gateway" ||
      id === "B8_gateway_early" ||
      id === "B3_system" ||
      id === "B2_hardware" ||
      id === "B12_anti_camouflage"
    ) {
      return false;
    }
    if (!primaryB10StillPending()) return false;
    // Exclusive gpu/audio/cpu/rtc packs (incl B10x, B18, B22, …) wait for primary B10.
    return true;
  }

  /** Merchant / lab custom probe registry (id → { resourceClasses, run, ... }). */
  var customProbes = Object.create(null);

  var EXCLUSIVE_CLASSES = ["gpu", "audio", "rtc", "cpu", "nest", "verify"];

  function normalizeClass(c) {
    c = String(c || "light").toLowerCase();
    if (c === "gl" || c === "webgl" || c === "webgpu") return "gpu";
    if (c === "webrtc" || c === "ice" || c === "network") return "rtc";
    if (EXCLUSIVE_CLASSES.indexOf(c) >= 0 || c === "light") return c;
    return "light";
  }

  /**
   * Primary resource class — mirrors brain_control::resource_class (compat).
   * Compound packs still report primary for brain stages; multi-lock uses resourceClasses().
   */
  function resourceClass(pOrId) {
    var classes = resourceClasses(pOrId);
    return classes[0] || "light";
  }

  /**
   * All exclusive classes a pack touches. Compound packs multi-lock so multi-source
   * brains never schedule B46 audio while B10 is still rendering OfflineAudio, etc.
   */
  function resourceClasses(pOrId) {
    var obj = typeof pOrId === "object" && pOrId ? pOrId : null;
    var id = typeof pOrId === "string" ? pOrId : String((obj && (obj.id || obj.pack_id || obj.batch_id)) || "");
    // Explicit pack / custom override
    if (obj) {
      if (Array.isArray(obj.resource_classes) && obj.resource_classes.length) {
        return uniqueClasses(obj.resource_classes.map(normalizeClass));
      }
      if (Array.isArray(obj.resourceClasses) && obj.resourceClasses.length) {
        return uniqueClasses(obj.resourceClasses.map(normalizeClass));
      }
      if (obj.resource_class || obj.resourceClass) {
        return [normalizeClass(obj.resource_class || obj.resourceClass)];
      }
    }
    if (id && customProbes[id]) {
      var cp = customProbes[id];
      if (Array.isArray(cp.resourceClasses) && cp.resourceClasses.length) {
        return uniqueClasses(cp.resourceClasses.map(normalizeClass));
      }
      if (cp.resourceClass) return [normalizeClass(cp.resourceClass)];
    }
    if (!id) return ["light"];
    if (/^R\d{2}/.test(id) || id.indexOf("spotcheck") >= 0) return ["verify"];
    if (id === "B7_sandbox") return ["nest"];
    // B10_hw_curves: single exclusive "gpu" lane (GA4/FingerprintJS style: critical
    // path must not require multi-lock AND of free lanes). Internal stages still run
    // audio→canvas→cpu→webgl sequentially inside the collector. Multi-lock
    // [gpu,audio,cpu] starved B10 forever when B2/B3 held cpu (lab: route_plan had
    // B10 but never uploaded). Secondary packs wait via mustDeferForPrimaryB10.
    if (id === "B10_hw_curves" || id === "mid.curves") return ["gpu"];
    if (
      id.indexOf("B10x_") === 0 ||
      id === "B15_cross_curves" ||
      id === "B18_webgpu" ||
      id === "B22_gpu_timer" ||
      id === "B23_native_canvas_hedge" ||
      id === "B30_gpu_bandwidth" ||
      id === "B84_gpu_bandwidth_ladder" ||
      id === "B31_shader_numeric" ||
      id === "B36_raster_msaa" ||
      id === "B57_canvas_emoji_path" ||
      id === "B58_canvas_text_metrics" ||
      id === "B59_webgl_params_full" ||
      id === "B60_webgl_extensions_full"
    ) {
      return ["gpu"];
    }
    if (id === "B46_audio_deep" || id === "B61_audio_worklet" || id === "B62_offline_audio_moments") {
      return ["audio"];
    }
    if (id === "B9_network" || id === "B55_webrtc_ice_deep") return ["rtc"];
    // iss/58 A3 WebCodecs — GPU encoder path
    if (id === "B81_webcodecs_bitstream") return ["gpu"];
    if (
      id === "B2_hardware" ||
      id === "B17_hw_physical" ||
      id === "B20_challenge_seed" ||
      id === "B33_caps_pressure" ||
      id === "B34_cpu_cache_ladder" ||
      id === "B37_thermal_drift_lite" ||
      id === "B42_thermal_drift_full" ||
      id === "B47_sab_clock" ||
      id === "B82_idb_write_ladder" ||
      id === "B83_eventloop_signature"
    ) {
      return ["cpu"];
    }
    // Custom-style ids: C##_… or custom_*
    if (/^C\d{2}/.test(id) || id.indexOf("custom_") === 0 || id.indexOf("merchant_") === 0) {
      var cl = id.toLowerCase();
      if (cl.indexOf("audio") >= 0) return ["audio"];
      if (cl.indexOf("webgl") >= 0 || cl.indexOf("gpu") >= 0 || cl.indexOf("canvas") >= 0) return ["gpu"];
      if (cl.indexOf("webrtc") >= 0 || cl.indexOf("ice") >= 0) return ["rtc"];
      if (cl.indexOf("cpu") >= 0 || cl.indexOf("hw") >= 0) return ["cpu"];
      return ["light"];
    }
    var l = id.toLowerCase();
    if (
      l.indexOf("webgl") >= 0 ||
      l.indexOf("webgpu") >= 0 ||
      l.indexOf("gpu") >= 0 ||
      l.indexOf("canvas") >= 0 ||
      l.indexOf("shader") >= 0 ||
      l.indexOf("raster") >= 0 ||
      l.indexOf("msaa") >= 0
    ) {
      return ["gpu"];
    }
    if (l.indexOf("audio") >= 0) return ["audio"];
    if (l.indexOf("webrtc") >= 0 || l.indexOf("ice") >= 0) return ["rtc"];
    if (l.indexOf("hw_") >= 0 || l.indexOf("thermal") >= 0 || l.indexOf("cpu") >= 0) return ["cpu"];
    return ["light"];
  }

  function uniqueClasses(arr) {
    var seen = Object.create(null);
    var out = [];
    var i;
    for (i = 0; i < arr.length; i++) {
      var c = normalizeClass(arr[i]);
      if (!c || c === "light") continue;
      if (seen[c]) continue;
      seen[c] = 1;
      out.push(c);
    }
    if (!out.length) out.push("light");
    // Stable acquire order prevents multi-lock deadlock across sources.
    out.sort(function (a, b) {
      return EXCLUSIVE_CLASSES.indexOf(a) - EXCLUSIVE_CLASSES.indexOf(b);
    });
    return out;
  }

  function isHardwarePack(pOrId) {
    var cs = resourceClasses(pOrId);
    return !(cs.length === 1 && cs[0] === "light");
  }

  /**
   * Register a merchant/lab custom probe pack.
   * @param {{id:string, resourceClass?:string, resourceClasses?:string[], run:Function, priority?:number, src?:string, batch_id?:string}} spec
   */
  function registerCustomProbe(spec) {
    if (!spec || !spec.id || typeof spec.run !== "function") {
      throw new Error("registerCustomProbe requires {id, run}");
    }
    var id = String(spec.id);
    var classes = [];
    if (Array.isArray(spec.resourceClasses) && spec.resourceClasses.length) {
      classes = uniqueClasses(spec.resourceClasses);
    } else if (spec.resourceClass) {
      classes = uniqueClasses([spec.resourceClass]);
    } else {
      classes = resourceClasses(id);
    }
    customProbes[id] = {
      id: id,
      resourceClasses: classes,
      resourceClass: classes[0] || "light",
      run: spec.run,
      priority: spec.priority != null ? Number(spec.priority) : 35,
      src: spec.src || null,
      batch_id: spec.batch_id || id,
      schedule: spec.schedule || "dynamic",
      layer: spec.layer || "custom",
    };
    // Also register into collector table when available (midEnqueue path).
    try {
      if (global.GRCollectors && typeof global.GRCollectors.register === "function") {
        global.GRCollectors.register(id, {
          priority: customProbes[id].priority,
          schedule: customProbes[id].schedule,
          batch_id: customProbes[id].batch_id,
          layer: customProbes[id].layer,
          resource_class: customProbes[id].resourceClass,
          resource_classes: customProbes[id].resourceClasses,
          run: function (ctx) {
            return customProbes[id].run(ctx);
          },
        });
      }
    } catch (eReg) {}
    try {
      global.__GR_CUSTOM_PROBES__ = customProbes;
    } catch (eG) {}
    return customProbes[id];
  }

  function listCustomProbes() {
    return Object.keys(customProbes).map(function (k) {
      return {
        id: k,
        resourceClasses: (customProbes[k].resourceClasses || []).slice(),
        priority: customProbes[k].priority,
      };
    });
  }

  /** Pack descriptor for kickAll from a registered custom probe. */
  function customProbePack(id) {
    var cp = customProbes[String(id || "")];
    if (!cp) return null;
    return {
      id: cp.id,
      pack_id: cp.id,
      batch_id: cp.batch_id,
      priority: cp.priority,
      schedule: cp.schedule,
      src: cp.src,
      resource_classes: cp.resourceClasses,
      resource_class: cp.resourceClass,
      run: cp.run,
    };
  }

  /**
   * Global ResourceBus: per-class exclusive lock shared by main + nest surfaces.
   * State on window so iframe/worker parent coordination uses same object.
   */
  function getBus() {
    if (!global.__GR_RESOURCE_BUS__) {
      global.__GR_RESOURCE_BUS__ = {
        holders: Object.create(null), // class -> { label, source, at, depth }
        waiters: Object.create(null), // class -> [{resolve, label}]
        nestDepth: Object.create(null), // class -> re-entrant depth on this turn
      };
    }
    return global.__GR_RESOURCE_BUS__;
  }

  function publishLockSnap() {
    try {
      var bus = getBus();
      var snap = {};
      Object.keys(bus.holders).forEach(function (c) {
        if (bus.holders[c]) snap[c] = bus.holders[c];
      });
      global.__GR_RESOURCE_LOCKS__ = snap;
      // Compat: any exclusive holder looks like "HW busy" for legacy nest waiters.
      var any = null;
      Object.keys(snap).forEach(function (c) {
        if (!any && snap[c] && c !== "light") any = snap[c];
      });
      global.__GR_HW_LOCK__ = any;
    } catch (eP) {}
  }

  function withResourceLock(cls, label, fn, source) {
    return withResourceLocks([cls], label, fn, source);
  }

  /**
   * Multi-class exclusive lock (sorted acquire → reverse release).
   * Prevents multi-source deadlock and same-hardware thrash for compound packs.
   */
  function withResourceLocks(classes, label, fn, source) {
    label = String(label || "pack");
    source = String(source || "main");
    var list = uniqueClasses(Array.isArray(classes) ? classes : [classes]);
    // Pure light: no exclusive lock
    if (list.length === 1 && list[0] === "light") {
      return Promise.resolve().then(fn);
    }
    // Drop light if mixed
    list = list.filter(function (c) {
      return c !== "light";
    });
    if (!list.length) {
      return Promise.resolve().then(fn);
    }

    var bus = getBus();

    // Re-entrant: if all requested classes already nested by this turn, nest deeper.
    var allNested = list.every(function (c) {
      return (bus.nestDepth[c] || 0) > 0;
    });
    if (allNested) {
      list.forEach(function (c) {
        bus.nestDepth[c] = (bus.nestDepth[c] || 0) + 1;
        try {
          if (bus.holders[c]) {
            bus.holders[c].nested = true;
            bus.holders[c].depth = bus.nestDepth[c];
          }
        } catch (eN) {}
      });
      publishLockSnap();
      return Promise.resolve()
        .then(fn)
        .then(
          function (r) {
            list.forEach(function (c) {
              bus.nestDepth[c]--;
            });
            publishLockSnap();
            return r;
          },
          function (e) {
            list.forEach(function (c) {
              bus.nestDepth[c]--;
            });
            publishLockSnap();
            throw e;
          }
        );
    }

    function acquireOne(cls) {
      return new Promise(function (resolve) {
        function tryTake() {
          if (!bus.holders[cls]) {
            bus.holders[cls] = {
              label: label,
              source: source,
              class: cls,
              at: Date.now(),
              depth: 1,
              multi: list.length > 1 ? list.slice() : undefined,
            };
            bus.nestDepth[cls] = 1;
            publishLockSnap();
            resolve();
            return;
          }
          bus.waiters[cls] = bus.waiters[cls] || [];
          bus.waiters[cls].push({ resolve: tryTake, label: label, source: source });
        }
        tryTake();
      });
    }

    function releaseOne(cls) {
      bus.nestDepth[cls] = 0;
      bus.holders[cls] = null;
      delete bus.holders[cls];
      publishLockSnap();
      var q = bus.waiters[cls] || [];
      if (q.length) {
        var next = q.shift();
        bus.waiters[cls] = q;
        setTimeout(function () {
          next.resolve();
        }, 0);
      }
    }

    // Acquire in stable order; release reverse on success/fail.
    var chain = Promise.resolve();
    list.forEach(function (c) {
      chain = chain.then(function () {
        return acquireOne(c);
      });
    });
    return chain
      .then(fn)
      .then(
        function (r) {
          for (var i = list.length - 1; i >= 0; i--) releaseOne(list[i]);
          return r;
        },
        function (e) {
          for (var j = list.length - 1; j >= 0; j--) releaseOne(list[j]);
          throw e;
        }
      );
  }

  /**
   * Hang recovery: force-drop exclusive class holders (e.g. B10 stuck on gpu)
   * and wake waiters so self-heal re-kick can acquire the lock.
   * Does NOT cancel in-flight JS; pair with GRGlGovernor.forceHardLoseForRetry.
   */
  function forceReleaseResource(cls, why) {
    cls = String(cls || "gpu");
    var bus = getBus();
    var had = !!(bus.holders && bus.holders[cls]);
    try {
      bus.nestDepth[cls] = 0;
      if (bus.holders) {
        bus.holders[cls] = null;
        delete bus.holders[cls];
      }
      var q = (bus.waiters && bus.waiters[cls]) || [];
      if (bus.waiters) bus.waiters[cls] = [];
      for (var i = 0; i < q.length; i++) {
        try {
          if (q[i] && typeof q[i].resolve === "function") q[i].resolve();
        } catch (eR) {}
      }
      try {
        if (typeof publishLockSnap === "function") publishLockSnap();
      } catch (eP) {}
      try {
        if (typeof GROps !== "undefined" && GROps.report) {
          GROps.report(
            "resource_force_release",
            "pack_loader",
            { class: cls, had: had, why: String(why || "") },
            "warn"
          );
        }
      } catch (eO) {}
    } catch (e) {}
    return had;
  }

  /** Backward-compat: treat as gpu class exclusive (legacy callers). */
  function withHardwareLock(label, fn) {
    return withResourceLock("gpu", label, fn, "main");
  }

  /** Wait until a resource class is free (nest coordination). */
  function waitResourceFree(cls, maxMs) {
    cls = String(cls || "gpu");
    maxMs = maxMs == null ? 8000 : maxMs;
    var t0 = Date.now();
    return new Promise(function (resolve) {
      function tick() {
        var bus = getBus();
        if (!bus.holders[cls]) {
          resolve(true);
          return;
        }
        if (Date.now() - t0 > maxMs) {
          resolve(false);
          return;
        }
        setTimeout(tick, 40);
      }
      tick();
    });
  }

  /**
   * Deterministic Fisher–Yates for multi-source stagger (mirrors brain shuffle_packs_for_source).
   */
  function shufflePacksForSource(packs, sessionId, source) {
    packs = (packs || []).slice();
    if (packs.length <= 1) return packs;
    var h = 0xcbf29ce484222325;
    var seed = String(sessionId || "") + "|" + String(source || "main");
    var i;
    for (i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i);
      h = Math.imul(h, 0x100000001b3);
    }
    for (i = packs.length - 1; i > 0; i--) {
      h ^= h << 13;
      h ^= h >>> 7;
      h ^= h << 17;
      h = h >>> 0;
      var j = h % (i + 1);
      var tmp = packs[i];
      packs[i] = packs[j];
      packs[j] = tmp;
    }
    return packs;
  }

  /**
   * Kick packs with ResourceBus concurrency:
   *  - light: up to maxLight
   *  - exclusive classes: 1 global each; different classes may parallel (gpu∥audio)
   */
  function kickAll(packs, ctx, opts) {
    if (probeHalted()) {
      return Promise.resolve([]);
    }
    opts = opts || {};
    var source = (ctx && (ctx.source || ctx.sandbox_kind)) || opts.source || "main";
    var sessionId =
      (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || global.__GR_CYCLE_ID__ || "";
    packs = (packs || []).slice();
    if (opts.shuffle !== false && packs.length > 1) {
      packs = shufflePacksForSource(packs, sessionId, source);
    } else {
      packs.sort(function (a, b) {
        return (b.priority || 0) - (a.priority || 0);
      });
    }
    // iss/62 P0 + multi-tab completeness:
    // Primary residual B10_hw_curves MUST lead before B10x deepen. kickAll admits
    // packs in array order and non-light classes serialize on the resource lock;
    // if B10x/gpu soft packs grab the lock first, multi-tab dwell often ends with
    // B10 still "running" and zero server B10 (probe-without-true-complete).
    // Order: B10_hw_curves → B10x must-land → rest (shuffle preserved for rest).
    if (packs.length > 1) {
      var b10Primary = [];
      var mlPacks = [];
      var otherPacks = [];
      for (var mli = 0; mli < packs.length; mli++) {
        var mlp = packs[mli];
        var mlId = String((mlp && (mlp.id || mlp.pack_id)) || "");
        var mlReason = String((mlp && mlp.reason) || "");
        if (mlId === "B10_hw_curves" || mlId === "mid.curves") {
          b10Primary.push(mlp);
        } else if (
          mlReason === "edh_b10x_silicon_must_land" ||
          mlId.indexOf("B10x_silicon_") === 0
        ) {
          mlPacks.push(mlp);
        } else {
          otherPacks.push(mlp);
        }
      }
      if (b10Primary.length || mlPacks.length) {
        mlPacks.sort(function (a, b) {
          return (b.priority || 0) - (a.priority || 0);
        });
        packs = b10Primary.concat(mlPacks, otherPacks);
      }
    }
    try {
      global.__GR_PACK_KICK_ORDER__ = (global.__GR_PACK_KICK_ORDER__ || []).concat(
        packs.map(function (p) {
          return { id: p.id || p.pack_id, source: source, class: resourceClass(p) };
        })
      );
    } catch (eKo) {}

    var maxLight = opts.maxLightConcurrent;
    // Adaptive policy from self-heal (engine-aware): prefer over static defaults.
    try {
      var ap = global.__GR_ADAPTIVE_POLICY__;
      if (ap && ap.max_light != null && maxLight == null) {
        maxLight = Number(ap.max_light);
      }
      if (ap && ap.serial_heavy) {
        global.__GR_PACK_MAX_CONCURRENT__ = 1;
      }
    } catch (eAp) {}
    if (maxLight == null && global.__GR_PACK_MAX_LIGHT__ != null) {
      maxLight = Number(global.__GR_PACK_MAX_LIGHT__);
    }
    if (maxLight == null || !(maxLight > 0)) {
      var mobile = false;
      var weak = false;
      try {
        var nav = global.navigator || {};
        var ua = String(nav.userAgent || "");
        mobile =
          /Mobi|Android|iPhone|iPad/i.test(ua) ||
          (nav.maxTouchPoints > 1 &&
            global.screen &&
            global.screen.width > 0 &&
            global.screen.width < 900);
        if (nav.deviceMemory != null && nav.deviceMemory <= 2) weak = true;
        if (nav.hardwareConcurrency != null && nav.hardwareConcurrency <= 2) weak = true;
        var conn = nav.connection || nav.mozConnection || nav.webkitConnection;
        if (conn && (conn.saveData || conn.effectiveType === "2g" || conn.effectiveType === "slow-2g")) {
          weak = true;
        }
        // Gecko/WebKit: default conservative (FPJS also serializes heavy entropy sources).
        var eng = (global.__GR_ADAPTIVE_POLICY__ && global.__GR_ADAPTIVE_POLICY__.engine) || "";
        if (eng === "gecko" || eng === "webkit") weak = true;
        if (typeof global.mozInnerScreenX === "number") weak = true;
        if (global.__GR_LAB_PRESSURE__ || (nav.hardwareConcurrency != null && nav.hardwareConcurrency <= 4)) {
          weak = true;
        }
      } catch (eM) {}
      maxLight = weak ? 1 : mobile ? 1 : 2;
    }
    maxLight = Math.max(1, Math.min(2, maxLight | 0));
    var forceSerial = opts.maxConcurrent === 1 || global.__GR_PACK_MAX_CONCURRENT__ === 1;

    try {
      global.__GR_LAST_KICK_CONC__ = {
        n: packs.length,
        max_light: maxLight,
        source: source,
        classes: packs.map(function (p) {
          return resourceClass(p);
        }),
        at: Date.now(),
      };
    } catch (eC) {}

    function classesForPack(p) {
      if (forceSerial) return ["gpu"];
      return resourceClasses(p);
    }

    function packTouchesGpu(classes) {
      return (classes || []).indexOf("gpu") >= 0;
    }

    function runOne(p) {
      var id = p.id || p.pack_id;
      var bid = p.batch_id || id;
      var classes = classesForPack(p);
      var cls = classes[0] || "light";
      setHealth(id, {
        kick: "started",
        priority: p.priority || 0,
        run: "pending",
        schedule: p.schedule || "static",
        resource_class: cls,
        resource_classes: classes.slice(),
        source: source,
        hardware: !(classes.length === 1 && classes[0] === "light"),
      });
      var chain = Promise.resolve();
      if (p.src) {
        chain = loadScript(p.src, id).catch(function (e) {
          setHealth(id, { load: "fail", error: String(e && e.message) });
        });
      } else {
        setHealth(id, { load: "inline" });
      }
      function execRun() {
        var allowH = allowKickDespiteHalt(id) || allowKickDespiteHalt(bid);
        if (probeHalted() && !allowH) {
          setHealth(id, { run: "skipped_halt" });
          return null;
        }
        // Cross-kickAll gate: do not run exclusive packs before primary B10 acks.
        if (mustDeferForPrimaryB10(id) || mustDeferForPrimaryB10(bid)) {
          setHealth(id, { run: "deferred_wait_b10" });
          try {
            if (typeof GROps !== "undefined" && GROps.report) {
              GROps.report("pack_deferred_b10", "kick", { id: String(id || "") }, "info");
            }
          } catch (eDef) {}
          // Clear kicked so self-heal / re-kick can retry after B10 lands.
          try {
            delete kicked[id];
            if (bid) delete kicked[bid];
          } catch (eK) {}
          return null;
        }
        // GPI multi-tab: non-heavy tabs must not run B10 / hard silicon (peer leader owns GPU).
        try {
          var isB10Pack =
            String(id || "") === "B10_hw_curves" ||
            String(id || "") === "mid.curves" ||
            String(bid || "") === "B10_hw_curves" ||
            String(bid || "") === "mid.curves";
          var isDeep =
            String(id || "").indexOf("B10x_") === 0 ||
            String(bid || "").indexOf("B10x_") === 0 ||
            id === "B18_webgpu" ||
            bid === "B18_webgpu";
          if (
            (isB10Pack || isDeep) &&
            global.GROriginCoordinator &&
            GROriginCoordinator.__ready &&
            typeof GROriginCoordinator.isHeavy === "function" &&
            !GROriginCoordinator.isHeavy()
          ) {
            // Allow if no live peer lease (sole tab / upgrade window)
            var allow = true;
            try {
              var gsnap = GROriginCoordinator.snapshot && GROriginCoordinator.snapshot();
              if (gsnap && gsnap.started && gsnap.lease && gsnap.lease.tab && gsnap.lease.tab !== gsnap.tab) {
                var lage = Date.now() - Number(gsnap.lease.hb || gsnap.lease.at || 0);
                if (lage < 6000) allow = false;
              }
            } catch (eLs) {}
            if (!allow) {
              setHealth(id, { run: "deferred_gpi_light", schedule: p.schedule || "static" });
              try {
                if (typeof GROps !== "undefined" && GROps.report) {
                  GROps.report(
                    "pack_deferred_gpi_light",
                    "kick",
                    { id: String(id || ""), bid: String(bid || "") },
                    "info"
                  );
                }
                if (global.__GR_DEBUG_GPI__ && global.console && console.info) {
                  console.info("[gr:pack] deferred_gpi_light", id || bid);
                }
              } catch (eOp) {}
              try {
                delete kicked[id];
                if (bid) delete kicked[bid];
              } catch (eK2) {}
              return null;
            }
          }
        } catch (eGpi) {}
        if (typeof p.run !== "function") {
          setHealth(id, { run: "noop_missing_run", schedule: p.schedule || "static" });
          try {
            if (typeof GROps !== "undefined" && GROps.report) {
              GROps.report("pack_no_run", "kick", { id: String(id || "") }, "warn");
            }
          } catch (eNo) {}
          return null;
        }
        setHealth(id, { run: "running", resource_class: cls, resource_classes: classes.slice() });
        var runMs = 22000;
        try {
          var ap = global.__GR_ADAPTIVE_POLICY__ || {};
          if (ap.engine === "webkit" || ap.engine === "gecko") runMs = 16000;
          if (isB10Pack) runMs = Math.min(runMs, Number(ap.b10_soft_stall_ms || runMs) + 2000);
        } catch (eTm) {}
        return Promise.resolve()
          .then(function () {
            if (probeHalted() && !allowH) return null;
            if (mustDeferForPrimaryB10(id) || mustDeferForPrimaryB10(bid)) return null;
            var runP = Promise.resolve().then(function () {
              return p.run(ctx);
            });
            return new Promise(function (resolve, reject) {
              var done = false;
              var to = setTimeout(function () {
                if (done) return;
                done = true;
                setHealth(id, { run: "timeout", error: "pack_run_timeout" });
                try {
                  forceReleaseResource("gpu", "pack_run_timeout:" + id);
                  forceReleaseResource("audio", "pack_run_timeout:" + id);
                  forceReleaseResource("cpu", "pack_run_timeout:" + id);
                } catch (eRel) {}
                resolve(null);
              }, runMs);
              runP.then(
                function (r) {
                  if (done) return;
                  done = true;
                  clearTimeout(to);
                  resolve(r);
                },
                function (err) {
                  if (done) return;
                  done = true;
                  clearTimeout(to);
                  reject(err);
                }
              );
            });
          })
          .then(function (r) {
            // Mark run-ok (not upload-ack). iss/70: upload fail does NOT clear kicked;
            // transport retries the same capture. Recollect only via quality_gap / collect-fail.
            kicked[id] = true;
            if (bid) kicked[bid] = true;
            setHealth(id, { run: "ok" });
            // Material Registry: collect success (transport owns ACK later).
            try {
              if (global.GRUploadQueue && GRUploadQueue.registryGet) {
                var srcC = source || "main";
                var prev = GRUploadQueue.registryGet(bid || id, srcC);
                if (!prev || prev.state !== "acked") {
                  // enqueue will freeze; stamp collected if freezeCapture available
                }
              }
            } catch (eReg) {}
            return r;
          })
          .catch(function (e) {
            setHealth(id, { run: "fail", error: String(e && e.message) });
            // iss/70: collect-side fail may allow re-kick.
            try {
              global.dispatchEvent(
                new CustomEvent("gr-collect-fail", {
                  detail: {
                    batch_id: bid || id,
                    pack_id: id,
                    error: String(e && e.message || e || ""),
                  },
                })
              );
            } catch (eCf) {}
            return null;
          });
      }
      return chain.then(function () {
        return withResourceLocks(classes, id, function () {
          return Promise.resolve()
            .then(execRun)
            .then(
              function (r) {
                // Soft-free GL after gpu pack (shared context reuse; hard lose is idle/pagehide).
                if (packTouchesGpu(classes)) {
                  try {
                    if (global.GRGlGovernor) {
                      if (GRGlGovernor.releaseSoft) GRGlGovernor.releaseSoft();
                      else if (GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
                    }
                  } catch (eG) {}
                }
                return r;
              },
              function (e) {
                if (packTouchesGpu(classes)) {
                  try {
                    if (global.GRGlGovernor) {
                      if (GRGlGovernor.releaseSoft) GRGlGovernor.releaseSoft();
                      else if (GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
                    }
                  } catch (eG2) {}
                }
                throw e;
              }
            );
        }, source);
      });
    }

    var results = new Array(packs.length);
    var next = 0;
    var activeLight = 0;
    var activeByClass = Object.create(null);
    var pending = packs.length;

    return new Promise(function (resolveAll) {
      if (!packs.length) {
        resolveAll([]);
        return;
      }
      function doneOne() {
        pending--;
        if (pending <= 0) resolveAll(results);
        else pump();
      }
      function classesFree(classes) {
        var k;
        for (k = 0; k < classes.length; k++) {
          var c = classes[k];
          if (c === "light") continue;
          if ((activeByClass[c] || 0) >= 1) return false;
        }
        return true;
      }

      function markClasses(classes, delta) {
        var k;
        for (k = 0; k < classes.length; k++) {
          var c = classes[k];
          if (c === "light") continue;
          activeByClass[c] = (activeByClass[c] || 0) + delta;
          if (activeByClass[c] < 0) activeByClass[c] = 0;
        }
      }

      function pump() {
        var busyEx = 0;
        Object.keys(activeByClass).forEach(function (c) {
          busyEx += activeByClass[c] || 0;
        });
        if (probeHalted() && activeLight === 0 && busyEx === 0 && next >= packs.length) {
          resolveAll(results);
          return;
        }
        while (next < packs.length) {
          var i = next;
          var p = packs[i];
          var classes = classesForPack(p);
          var cls = classes[0] || "light";
          var isLight = classes.length === 1 && classes[0] === "light";
          // DAG v2 dependency gate: defer once when a declared dependency is not
          // kicked yet, not acked, and not scheduled earlier in this runnable.
          // Swap a later ready pack into this slot; if the queue is all dependent
          // the blocked-head re-pump timer wakes us (dep stays soft after one try).
          if (dagDepUnmet(p, packs, next, null, true)) {
            if (p) p._dagDepTried = true;
            var foundDep = -1;
            var jd2;
            for (jd2 = next + 1; jd2 < packs.length; jd2++) {
              if (dagDepUnmet(packs[jd2], packs, next, null, false)) continue;
              var c2sD = classesForPack(packs[jd2]);
              var c2LightD = c2sD.length === 1 && c2sD[0] === "light";
              if (c2LightD) {
                if (activeLight < maxLight) {
                  foundDep = jd2;
                  break;
                }
                continue;
              }
              if (classesFree(c2sD)) {
                foundDep = jd2;
                break;
              }
            }
            if (foundDep < 0) break;
            var tmpDep = packs[next];
            packs[next] = packs[foundDep];
            packs[foundDep] = tmpDep;
            publishDagSnapshot();
            continue;
          }
          if (isLight) {
            if (activeLight >= maxLight) break;
            next++;
            activeLight++;
            (function (idx, pack) {
              Promise.resolve()
                .then(function () {
                  return runOne(pack);
                })
                .then(
                  function (r) {
                    results[idx] = r;
                  },
                  function () {
                    results[idx] = null;
                  }
                )
                .then(function () {
                  activeLight--;
                  doneOne();
                });
            })(i, p);
            continue;
          }
          // Global: exclusive non-B10 packs wait until primary B10 acked.
          var packIdNow = String((p && (p.id || p.pack_id)) || "");
          if (mustDeferForPrimaryB10(packIdNow)) {
            // Skip this slot for now; try later packs that are light / allowed.
            var foundDef = -1;
            var jd;
            for (jd = next + 1; jd < packs.length; jd++) {
              var pidD = String((packs[jd] && (packs[jd].id || packs[jd].pack_id)) || "");
              if (mustDeferForPrimaryB10(pidD)) continue;
              var cDs = classesForPack(packs[jd]);
              var cDLight = cDs.length === 1 && cDs[0] === "light";
              if (cDLight) {
                if (activeLight < maxLight) {
                  foundDef = jd;
                  break;
                }
                continue;
              }
              if (classesFree(cDs)) {
                foundDef = jd;
                break;
              }
            }
            if (foundDef < 0) break;
            var tmpD = packs[next];
            packs[next] = packs[foundDef];
            packs[foundDef] = tmpD;
            continue;
          }
          if (!classesFree(classes)) {
            // Head-of-line is blocked. Pull only *light* packs ahead — never
            // jump exclusive packs over primary residual B10_hw_curves.
            // Lab bug: reordering free gpu/cpu packs ahead of B10 starved
            // multi-lock B10 forever (route_plan had B10 but server never saw it).
            var headId = String((p && (p.id || p.pack_id)) || "");
            var headIsB10 = headId === "B10_hw_curves" || headId === "mid.curves";
            var found = -1;
            var j;
            for (j = next + 1; j < packs.length; j++) {
              var c2s = classesForPack(packs[j]);
              var c2Light = c2s.length === 1 && c2s[0] === "light";
              if (c2Light) {
                if (activeLight < maxLight) {
                  found = j;
                  break;
                }
                continue;
              }
              // While primary B10 waits for locks, do NOT run other exclusive packs.
              if (headIsB10) continue;
              if (mustDeferForPrimaryB10(packs[j])) continue;
              if (classesFree(c2s)) {
                found = j;
                break;
              }
            }
            if (found < 0) {
              // Exclusive head (often B10 multi-lock) is waiting. Re-pump shortly
              // instead of abandoning the rest of the queue forever.
              pump.waitN = (pump.waitN || 0) + 1;
              if (pump.waitN === 1 || pump.waitN % 25 === 0) {
                try {
                  if (typeof GROps !== "undefined" && GROps.report) {
                    GROps.report(
                      "pack_bus_wait",
                      "kick",
                      {
                        head: headId,
                        waits: pump.waitN,
                        active: Object.assign({}, activeByClass),
                      },
                      pump.waitN >= 50 ? "warn" : "info"
                    );
                  }
                  var tl = global.__GR_PROBE_TIMELINE__;
                  if (Array.isArray(tl)) {
                    tl.push({
                      t: Date.now(),
                      ev: "pack_bus_wait",
                      d: { head: headId, waits: pump.waitN },
                    });
                    if (tl.length > 80) tl.splice(0, tl.length - 80);
                  }
                } catch (eW) {}
              }
              // Long bus wait → ask self-heal to recover (smart, not infinite wait).
              if (pump.waitN === 40 && headIsB10) {
                try {
                  if (
                    global.GRProbeSelfHeal &&
                    typeof GRProbeSelfHeal.recoverFromStall === "function"
                  ) {
                    var gWait =
                      GRProbeSelfHeal.ensureGap &&
                      GRProbeSelfHeal.ensureGap("B10_hw_curves", "main");
                    if (gWait) GRProbeSelfHeal.recoverFromStall(gWait, "pack_bus_wait");
                  }
                } catch (eRec) {}
              }
              if (!pump.timer) {
                pump.timer = setTimeout(function () {
                  pump.timer = null;
                  pump();
                }, 120);
              }
              break;
            }
            var tmp = packs[next];
            packs[next] = packs[found];
            packs[found] = tmp;
            continue;
          }
          next++;
          markClasses(classes, 1);
          (function (idx, pack, cclsList) {
            Promise.resolve()
              .then(function () {
                return runOne(pack);
              })
              .then(
                function (r) {
                  results[idx] = r;
                },
                function () {
                  results[idx] = null;
                }
              )
              .then(function () {
                markClasses(cclsList, -1);
                doneOne();
              });
          })(i, p, classes.slice());
        }
      }
      pump();
    });
  }

  function planHasRandomVerify(routePlan) {
    var packs = (routePlan && routePlan.packs) || [];
    for (var i = 0; i < packs.length; i++) {
      var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
      if (id.indexOf("spotcheck") >= 0 || /^R\d{2}/.test(id)) return true;
    }
    return false;
  }

  /* ================= probe_dag_v2 scheduler (design §9.3) =================
   *
   * The FE consumes the server-issued DAG v2 metadata (route_plan packs carry
   * dag_v2 = { planes, depends_on, resource, deadline_ms, engine_profiles,
   * cost_class, source_kind, commercial_roles, diagnostic_roles, fallbacks,
   * missing_state, replay_binding }).
   *
   * Decisions made here are OBSERVABLE: every skipped pack gets a reason in
   * __GR_DAG_V2_CONSUMED__.skipped, the engine decision is a feature-claim
   * (UA recorded separately, weight ~0), and both the dag version and the
   * engine claim enter the sealed pack-set hash so the server can later answer
   * "why was field X not executed" from the sealed envelope alone.
   */

  /** DAG v2 engine decision: feature evidence first, UA only as a claim. */
  function dagEngineDecision() {
    var ua = "";
    try {
      ua = String(global.navigator && navigator.userAgent || "");
    } catch (eUa) {}
    var classified = "unknown";
    try {
      // Capability evidence only. UA/brands remain claims and never select a
      // hard gate. Require two independent signals before narrowing.
      var scores = { gecko: 0, blink: 0, webkit: 0 };
      if (typeof global.mozInnerScreenX === "number") scores.gecko++;
      if (global.CSS && typeof global.CSS.supports === "function" &&
          global.CSS.supports("-moz-appearance", "none")) scores.gecko++;
      if (global.chrome && global.chrome.runtime) scores.blink++;
      if (global.navigator && global.navigator.userAgentData) scores.blink++;
      if (global.WebKitCSSMatrix) scores.webkit++;
      if (typeof global.webkitRequestAnimationFrame === "function") scores.webkit++;
      if (global.webkitAudioContext) scores.webkit++;
      Object.keys(scores).forEach(function (engine) {
        if (scores[engine] >= 2 && scores[engine] > (scores[classified] || 0)) classified = engine;
      });
    } catch (eD) {}
    return { classified: classified, ua_claim: ua };
  }

  /** Best-effort Method Matrix engine (same evidence, single source where ready). */
  function dagEngineFromMatrix() {
    try {
      if (global.GRProbeMethodMatrix && typeof global.GRProbeMethodMatrix.detectEngine === "function") {
        var e = global.GRProbeMethodMatrix.detectEngine();
        if (e && e !== "unknown") return e;
      }
    } catch (eM) {}
    return null;
  }

  var dagState = {
    version: 0,
    engine: "unknown",
    ua_claim: "",
    skipped: Object.create(null),
    dep_wait: Object.create(null),
    deadlines: Object.create(null),
    plan_at: 0,
    consumed: false,
  };

  /** Attach per-pack dag_v2 (route_plan packs carry it; self-heal re-kicks may not). */
  function dagV2For(p) {
    var d = (p && p.dag_v2) || null;
    if (d && typeof d === "object") return d;
    // Fall back to a minimal descriptor so gates degrade to permissive.
    return null;
  }

  /** Engine gate: does this pack run on the classified engine? */
  function dagEngineAllows(d, engine) {
    if (!d || !Array.isArray(d.engine_profiles)) return true;
    if (engine === "unknown") return true; // never silently drop on uncertainty
    return d.engine_profiles.indexOf(engine) >= 0;
  }

  function dagRecordSkip(id, reason) {
    try {
      dagState.skipped[id] = reason;
      setHealth(id, { dag_skip: reason });
      if (typeof GROps !== "undefined" && GROps.report) {
        GROps.report("pack_dag_skip", "dag_v2", { pack: String(id || ""), reason: reason }, "info");
      }
    } catch (eS) {}
  }

  /**
   * Filter runnable packs through the DAG v2 engine gate. Packs excluded by the
   * engine profile stay un-kicked and un-kicked flags are NOT set, so a later
   * re-plan (or engine reclassification) can still run them.
   */
  function dagApplyGate(runnable, routePlan) {
    var dagVersion = ((routePlan && routePlan.dag_version) || 0) | 0;
    var hasDag = dagVersion >= 2 || (runnable || []).some(function (r) {
      return !!dagV2For(r);
    });
    if (!hasDag) return runnable;
    var engine = dagEngineFromMatrix() || dagEngineDecision().classified;
    dagState.version = Math.max(dagState.version, dagVersion);
    dagState.engine = engine;
    dagState.ua_claim = dagEngineDecision().ua_claim;
    dagState.consumed = true;
    var planAt = Date.now();
    if (!dagState.plan_at) dagState.plan_at = planAt;
    var out = [];
    for (var i = 0; i < runnable.length; i++) {
      var p = runnable[i];
      var id = String((p && (p.id || p.pack_id || p.batch_id)) || "");
      var d = dagV2For(p);
      if (d && !dagEngineAllows(d, engine)) {
        dagRecordSkip(id, "engine_profile_excluded:" + engine);
        continue;
      }
      // B-batch four-piece gate: research lanes (B18 webgpu / B47 SAB) stay out
      // of the default schedule until their dual-KPI gate clears. Server-side
      // route_plan already excludes them; this is the FE default-admission floor.
      if (d && (d.gate === "research" || d.dual_kpi_gate === "research")) {
        dagRecordSkip(id, "gate_research_hold");
        continue;
      }
      if (d && d.deadline_ms) {
        var baseDeadline = Number(d.deadline_ms) || 0;
        // Gecko/WebKit get a longer soft deadline (design §9.2).
        var effDeadline = (engine === "gecko" || engine === "webkit") ? Math.round(baseDeadline * 1.25) : baseDeadline;
        dagState.deadlines[id] = {
          deadline_ms: effDeadline,
          planned_at: dagState.plan_at,
          elapsed_ms: planAt - dagState.plan_at,
          cost_class: d.cost_class || "light",
        };
        if (effDeadline > 0 && planAt - dagState.plan_at > effDeadline) {
          dagRecordSkip(id, "deadline_exceeded_degrade:" + effDeadline);
          // Deadline exceeded: still collect (honest_skip/fallback is a server
          // analysis decision), but demote priority so fresh packs lead.
          if (p && p.priority) p.priority = Math.max(1, Math.round(Number(p.priority) / 2));
        }
      }
      out.push(p);
    }
    publishDagSnapshot();
    return out;
  }

  /**
   * Dependency gate inside the kickAll pump: a pack whose dag_v2.depends_on ids
   * are not (a) already kicked, (b) acked by transport, nor (c) present earlier
   * in this runnable list is deferred once — the pump swaps a later ready pack
   * into its slot. If no ready pack exists the head stays and the bus-wait timer
   * re-pumps. One re-pump is enough; afterwards the pack runs anyway (deps are
   * soft unless the server marks them hard in the future).
   */
  function dagDepUnmet(p, runnable, next, kickedNow, record) {
    record = record !== false;
    var d = dagV2For(p);
    if (!d || !Array.isArray(d.depends_on) || !d.depends_on.length) return false;
    var id = String((p && (p.id || p.pack_id || p.batch_id)) || "");
    if (p && p._dagDepTried) return false; // already deferred once this kick
    for (var i = 0; i < d.depends_on.length; i++) {
      var dep = String(d.depends_on[i]);
      if ((kickedNow && kickedNow[dep]) || kicked[dep]) continue; // kicked already
      var depPresent = false;
      for (var j = 0; j < next; j++) {
        var q = runnable[j];
        if (!q) continue;
        var qid = String((q && (q.id || q.pack_id || q.batch_id)) || "");
        if (qid === dep) {
          depPresent = true;
          break;
        }
      }
      if (depPresent) continue;
      // Dependency not kicked, not acked, not scheduled before us → defer once.
      if (record) {
        dagState.dep_wait[id] = { depends_on: dep, tried: 1 };
        try {
          setHealth(id, { dag_dep_wait: dep });
          if (typeof GROps !== "undefined" && GROps.report) {
            GROps.report("pack_dag_dep_wait", "dag_v2", { pack: id, depends_on: dep }, "info");
          }
        } catch (eD) {}
      }
      return true;
    }
    return false;
  }

  function publishDagSnapshot() {
    try {
      global.__GR_DAG_V2_CONSUMED__ = {
        version: dagState.version,
        engine: dagState.engine,
        ua_claim: dagState.ua_claim,
        skipped: Object.assign({}, dagState.skipped),
        dep_wait: Object.assign({}, dagState.dep_wait),
        deadlines: Object.assign({}, dagState.deadlines),
        plan_at: dagState.plan_at,
        consumed: dagState.consumed,
        at: Date.now(),
      };
    } catch (eP) {}
  }

  function dagSnapshot() {
    publishDagSnapshot();
    return global.__GR_DAG_V2_CONSUMED__ || {};
  }

  /** Compact stamp for uploads / seal binding (no full skip map on every batch). */
  function dagUploadStamp() {
    var s = dagSnapshot();
    var skippedN = 0;
    Object.keys(s.skipped || {}).forEach(function (k) {
      skippedN++;
    });
    return {
      dag_v2_version: s.version || 0,
      dag_v2_engine: s.engine || "unknown",
      dag_v2_skipped_n: skippedN,
      dag_v2_consumed: !!s.consumed,
    };
  }

  function resetDag() {
    dagState.version = 0;
    dagState.skipped = Object.create(null);
    dagState.dep_wait = Object.create(null);
    dagState.deadlines = Object.create(null);
    dagState.plan_at = Date.now();
    dagState.consumed = false;
  }

  /**
   * Before applying route_plan: load only collector layers + R packs that the plan needs
   * (static already loaded; mid/dense/R on demand). Enables edge-load + edge-upload.
   * Protocol 2: prefer manifest.layers URLs so VERSION bumps always hit new paths.
   */
  function ensureCollectorsForPlan(routePlan) {
    var needHard = false;
    var needMid = false;
    var needDense = false;
    var needB10x = false;
    try {
      var packs = (routePlan && routePlan.packs) || [];
      for (var i = 0; i < packs.length; i++) {
        var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
        if (!id) continue;
        if (id === "B10_hw_curves" || id === "B7_sandbox" || id.indexOf("B1") === 0) needHard = true;
        if (id.indexOf("B10x_") === 0) needB10x = true;
        if (/^B(1[5-9]|[2-4][0-9]|[5-9])/.test(id) && id.indexOf("B10") !== 0) needMid = true;
        if (id.indexOf("B4") === 0 || id.indexOf("B5") === 0 || id.indexOf("B6") === 0 || id.indexOf("B7") === 0) {
          if (id !== "B7_sandbox") needDense = true;
        }
        if (id === "B10_hw_curves" || id === "B2_hardware" || id === "B7_sandbox") needHard = true;
      }
    } catch (eNeed) {}

    var chain = Promise.resolve();
    function loadLayer(name) {
      var url = layerUrl(name);
      if (!url) return Promise.resolve("no_" + name);
      return loadScript(url, "layer_" + name).catch(function () {
        return "fail_" + name;
      });
    }
    // Prefetch hard early when plan mentions residual packs
    if (needHard) chain = chain.then(function () { return loadLayer("hard"); });
    if (needB10x) chain = chain.then(function () { return loadLayer("b10x"); });
    if (needMid) chain = chain.then(function () { return loadLayer("mid"); });
    if (needDense) chain = chain.then(function () { return loadLayer("dense"); });

    return chain
      .then(function () {
        if (typeof global.ensureModulesForRoutePlan === "function") {
          return global.ensureModulesForRoutePlan(routePlan).catch(function () {
            return "mod_plan_fail";
          });
        }
        return ensureRandomIfNeeded(routePlan);
      });
  }

  /**
   * Ensure R runtime + only the R packs listed in route_plan (usually 1).
   * Does NOT load all 100 packs — each pack is its own small minified script.
   */
  function ensureRandomIfNeeded(routePlan) {
    if (!planHasRandomVerify(routePlan)) return Promise.resolve("no_r");
    if (typeof global.ensureRandomVerifyPacksForPlan === "function") {
      return global.ensureRandomVerifyPacksForPlan(routePlan).catch(function () {
        return "r_plan_fail";
      });
    }
    if (typeof global.ensureRandomVerifyModules === "function") {
      return global.ensureRandomVerifyModules().then(function () {
        var packs = (routePlan && routePlan.packs) || [];
        var jobs = [];
        for (var i = 0; i < packs.length; i++) {
          var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
          if (/^R\d{2}_spotcheck$/.test(id) && typeof global.ensureRandomVerifyPack === "function") {
            jobs.push(global.ensureRandomVerifyPack(id));
          }
        }
        return jobs.length ? Promise.all(jobs) : "r_rt_only";
      });
    }
    return Promise.resolve("r_no_loader");
  }

  /**
   * Apply brain route_plan: kick only packs not already kicked.
   * Returns { kicked: string[], skipped: string[], stop_probe: bool, plan_version }
   */
  function applyRoutePlan(routePlan, ctx) {
    if (probeHalted()) {
      return Promise.resolve({
        kicked: [],
        skipped: [],
        stop_probe: true,
        plan_version: (routePlan && routePlan.plan_version) || 0,
        halted: true,
      });
    }
    if (!routePlan) {
      return Promise.resolve({ kicked: [], skipped: [], stop_probe: true, plan_version: 0 });
    }
    if (routePlan.stop_probe && (!routePlan.packs || !routePlan.packs.length)) {
      global.__GR_STOP_PROBE__ = true;
      try {
        var Qh = global.GRUploadQueue;
        if (Qh && Qh.halt) {
          Qh.halt("stop_probe", (ctx && ctx.session_id) || global.__GR_SESSION_ID__, "stop_probe");
        }
      } catch (eH) {}
      return Promise.resolve({
        kicked: [],
        skipped: [],
        stop_probe: true,
        plan_version: routePlan.plan_version || 0,
      });
    }
    // Load mid/dense/R layers required by this plan only (not bulk all B packs).
    return ensureCollectorsForPlan(routePlan).then(function () {
      return applyRoutePlanAfterRandom(routePlan, ctx);
    });
  }

  function applyRoutePlanAfterRandom(routePlan, ctx) {
    var Col = global.GRCollectors;
    if (!Col || !Col.packsFromRoutePlan) {
      return Promise.resolve({
        kicked: [],
        skipped: [],
        stop_probe: !!routePlan.stop_probe,
        error: "no_collectors",
        plan_version: routePlan.plan_version || 0,
      });
    }
    // Allow re-run of dynamic packs not yet uploaded (kicked flag alone is insufficient).
    // force_recollect=true (brain stack_residual_missing) re-kicks even when already
    // kicked/sent so residual fields can be filled on an existing B2/B10 batch.
    var Q = global.GRUploadQueue;
    var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
    var softSkip = Object.assign({}, kicked);
    var forceMap = Object.create(null);
    var clearSent = [];
    (routePlan.packs || []).forEach(function (p) {
      var id = p.pack_id || p.id;
      var bid = p.batch_id || id;
      var src = p.source || "main";
      if (!id) return;
      var force = p.force_recollect === true;
      if (force) {
        // iss/65: refuse force_recollect storm — at most one clear+re-upload per batch/session
        // after terminal sealed_ok (unless queue says materials incomplete).
        var allowForce = true;
        try {
          if (
            Q &&
            Q.alreadySent &&
            sid &&
            bid &&
            Q.alreadySent({ session_id: sid, batch_id: bid, source: src })
          ) {
            if (Q.allowForceRecollect) {
              allowForce = !!Q.allowForceRecollect(bid);
            } else {
              // Fallback: one force per batch after first success.
              global.__GR_FORCE_RECOLLECT_N__ = global.__GR_FORCE_RECOLLECT_N__ || Object.create(null);
              var fk = String(sid) + "|" + String(bid);
              var n = global.__GR_FORCE_RECOLLECT_N__[fk] || 0;
              if (n >= 1) allowForce = false;
              else global.__GR_FORCE_RECOLLECT_N__[fk] = n + 1;
            }
          }
        } catch (eForce) {}
        if (!allowForce) {
          // Treat as normal already-sent skip.
          return;
        }
        forceMap[id] = true;
        if (bid) forceMap[bid] = true;
        delete softSkip[id];
        if (bid) delete softSkip[bid];
        // Also clear sticky kicked so parallel apply paths see a clean slate.
        delete kicked[id];
        if (bid) delete kicked[bid];
        clearSent.push({ session_id: sid, batch_id: bid, source: src });
        return;
      }
      // Non-force: if not alreadySent, allow mid retry despite sticky kick.
      if (Q && Q.alreadySent && sid) {
        if (!Q.alreadySent({ session_id: sid, batch_id: bid, source: src })) {
          delete softSkip[id];
          delete softSkip[bid];
        }
      }
    });
    if (Q && Q.clearSentKeys && clearSent.length) {
      Q.clearSentKeys(clearSent);
    }
    var runnable = Col.packsFromRoutePlan(routePlan, softSkip);
    // DAG v2 engine gate: drop packs the classified engine profile excludes and
    // record the reason (observable in __GR_DAG_V2_CONSUMED__.skipped and in
    // the sealed pack-set decision).
    runnable = dagApplyGate(runnable, routePlan);
    var allIds = (routePlan.packs || []).map(function (p) {
      return p.pack_id || p.id;
    });
    var skipped = allIds.filter(function (id) {
      return dagState.skipped[id] || softSkip[id] || !runnable.some(function (r) {
        return r.id === id || r.pack_id === id;
      });
    });
    if (!runnable.length) {
      return Promise.resolve({
        kicked: [],
        skipped: skipped,
        stop_probe: !!routePlan.stop_probe,
        plan_version: routePlan.plan_version || 0,
        force_recollect: Object.keys(forceMap),
      });
    }
    var runCtx = ctx || {};
    if (Object.keys(forceMap).length) {
      runCtx = Object.assign({}, ctx || {}, { force_recollect_batches: forceMap });
    }
    // Staged multi-source sandbox from brain (wave 0..2 kinds / max_concurrent)
    if (routePlan.sandbox_plan) {
      runCtx = Object.assign({}, runCtx, { sandbox_plan: routePlan.sandbox_plan });
      try {
        global.__GR_SANDBOX_PLAN__ = routePlan.sandbox_plan;
      } catch (eSp) {}
    }
    // Prefer brain parallel_groups for staged race; default = all concurrent (v57).
    var groups = routePlan.parallel_groups;
    var kickP;
    if (groups && groups.length > 1) {
      // Sequential groups, parallel within each (staged core→mid).
      kickP = Promise.resolve();
      groups.forEach(function (gids) {
        kickP = kickP.then(function () {
          var subset = runnable.filter(function (r) {
            var id = r.id || r.pack_id;
            return gids.indexOf(id) >= 0;
          });
          if (!subset.length) return null;
          return kickAll(subset, runCtx);
        });
      });
      // Packs not listed in any group still kick in parallel at end.
      kickP = kickP.then(function () {
        var listed = {};
        groups.forEach(function (g) {
          (g || []).forEach(function (id) {
            listed[id] = true;
          });
        });
        var rest = runnable.filter(function (r) {
          return !listed[r.id] && !listed[r.pack_id];
        });
        return rest.length ? kickAll(rest, runCtx) : null;
      });
    } else {
      // Single group / empty groups: full parallel race.
      kickP = kickAll(runnable, runCtx);
    }
    return kickP.then(function () {
      global.__GR_LAST_DYNAMIC_KICK__ = {
        at: Date.now(),
        packs: runnable.map(function (r) {
          return r.id;
        }),
        plan_version: routePlan.plan_version || 0,
        force_recollect: Object.keys(forceMap),
        parallel_groups: groups || [],
      };
      return {
        kicked: runnable.map(function (r) {
          return r.id;
        }),
        skipped: skipped,
        stop_probe: !!routePlan.stop_probe,
        plan_version: routePlan.plan_version || 0,
        force_recollect: Object.keys(forceMap),
        parallel_groups: groups || [],
      };
    });
  }

  function alreadyKicked() {
    return Object.assign({}, kicked);
  }

  function markKicked(id) {
    kicked[id] = true;
  }

  function clearKicked(id) {
    if (id == null || id === "") return;
    delete kicked[String(id)];
  }

  function resetKicked() {
    kicked = Object.create(null);
  }

  /**
   * iss/70 P0 / architecture synthesis R1:
   * Upload failure must NOT clear kicked → re-collect GPU/CPU/Audio.
   * Transport retries keep the same capture (upload_queue.scheduleRetry + payload).
   * Recollect only when explicitly requested:
   *   - detail.recollect / force_recollect / action=recollect / quality_gap
   *   - detail.collect_fail / payload_invalid (material bad, not network)
   *   - separate event gr-collect-fail
   */
  function maybeClearKickedForRecollect(d) {
    d = d || {};
    var bid = String(d.batch_id || "");
    if (!bid) return false;
    var want =
      d.recollect === true ||
      d.force_recollect === true ||
      d.forceRecollect === true ||
      String(d.action || "") === "recollect" ||
      d.quality_gap === true ||
      d.collect_fail === true ||
      d.payload_invalid === true ||
      String(d.reason || "") === "quality_gap" ||
      String(d.reason || "") === "collect_fail";
    if (!want) return false;
    clearKicked(bid);
    if (d.pack_id) clearKicked(String(d.pack_id));
    return true;
  }

  try {
    if (!global.__GR_PACK_FAIL_HOOK__) {
      global.__GR_PACK_FAIL_HOOK__ = true;
      // Network/HTTP/seal transport fails: telemetry only (do not clearKicked).
      global.addEventListener("gr-upload-fail", function (ev) {
        try {
          maybeClearKickedForRecollect((ev && ev.detail) || {});
        } catch (eF) {}
      });
      // Collect-side failure: allow re-kick under pack budget / scheduler.
      global.addEventListener("gr-collect-fail", function (ev) {
        try {
          var d = (ev && ev.detail) || {};
          d.collect_fail = true;
          maybeClearKickedForRecollect(d);
        } catch (eC) {}
      });
      // Explicit quality-gap / server recollect signal.
      global.addEventListener("gr-quality-gap", function (ev) {
        try {
          var d = (ev && ev.detail) || {};
          d.quality_gap = true;
          d.recollect = d.recollect !== false;
          maybeClearKickedForRecollect(d);
        } catch (eQ) {}
      });
    }
  } catch (eHook) {}

  global.GRPackLoader = {
    kickAll: kickAll,
    applyRoutePlan: applyRoutePlan,
    ensureCollectorsForPlan: ensureCollectorsForPlan,
    ensureRandomIfNeeded: ensureRandomIfNeeded,
    health: function () {
      return health;
    },
    loadScript: loadScript,
    alreadyKicked: alreadyKicked,
    markKicked: markKicked,
    clearKicked: clearKicked,
    resetKicked: resetKicked,
    isHardwarePack: isHardwarePack,
    resourceClass: resourceClass,
    resourceClasses: resourceClasses,
    withHardwareLock: withHardwareLock,
    withResourceLock: withResourceLock,
    withResourceLocks: withResourceLocks,
    forceReleaseResource: forceReleaseResource,
    waitResourceFree: waitResourceFree,
    shufflePacksForSource: shufflePacksForSource,
    getResourceBus: getBus,
    registerCustomProbe: registerCustomProbe,
    listCustomProbes: listCustomProbes,
    customProbePack: customProbePack,
    withProductVer: withProductVer,
    layerUrl: layerUrl,
    assetBase: assetBase,
    manifest: manifest,
    // probe_dag_v2 scheduler surface (design §9.3)
    dagEngineDecision: dagEngineDecision,
    dagSnapshot: dagSnapshot,
    dagUploadStamp: dagUploadStamp,
    dagReset: resetDag,
  };
})(typeof window !== "undefined" ? window : globalThis);

/* ---- collectors/l1.js ---- */
/**
 * GR L1 — ultra-lite short-visit collectors (v57 mp.l1 aligned).
 * Flat `fields` payloads for greenv5 brain (not nested max-probe sections).
 * Independent of collectors/registry.js — first evidence without 200KB+ module.
 *
 * Public API (stable under minify):
 *   GRL1.collectB0() / collectB1() / collectB2() / collectB3() / collectB12()
 *   GRL1.enqueueBatch(ctx, batchId, fields, priority)
 *   GRL1.runB0(ctx) → Promise
 *   GRL1.runProgressiveRest(ctx) → Promise  // B1,B12,B2,B3 after B0
 *   GRL1.analysisCriticalOrder / priorities / batchIds
 */
(function (global) {
  "use strict";

  function safe(fn, fb) {
    try {
      return fn();
    } catch (e) {
      return fb !== undefined ? fb : null;
    }
  }

  function deriveOsFamily(ua, platform) {
    ua = String(ua || "").toLowerCase();
    platform = String(platform || "").toLowerCase();
    if (/android/.test(platform)) return "android";
    if (/iphone|ipad|ipod/.test(platform)) return "ios";
    if (/win/.test(platform)) return "windows";
    if (/linux|x11/.test(platform)) return "linux";
    if (/mac/.test(platform)) return "macos";
    if (/android/.test(ua)) return "android";
    if (/iphone|ipad|ipod|ios/.test(ua)) return "ios";
    if (/cros/.test(ua)) return "chromeos";
    if (/windows|win32|win64/.test(ua)) return "windows";
    if (/linux|x11/.test(ua)) return "linux";
    if (/mac os x|macintosh|macintel/.test(ua)) return "macos";
    if (platform) return "other";
    return "";
  }

  function formClass(scr, nav) {
    var w = (scr && scr.width) || 0;
    var touch = (nav && nav.maxTouchPoints) || 0;
    if (w > 0 && w < 600) return "mobile";
    if (touch > 1 && w > 0 && w < 900) return "mobile";
    var ua = (nav && nav.userAgent) || "";
    if (/Mobi|Android|iPhone|iPad/i.test(ua) && (touch > 0 || w < 900)) return "mobile";
    return "desktop";
  }

  /**
   * Engine family without InstallTrigger (deprecated in Firefox).
   * Capability-first: chrome object → blink; mozInnerScreenX / real Gecko UA → gecko.
   */
  function engineFamily(ua) {
    ua = ua || "";
    if (!!(global.chrome) || /Chrome\//.test(ua) || /CriOS\//.test(ua) || /Edg\//.test(ua) || /OPR\//.test(ua))
      return "blink";
    try {
      if (typeof global.mozInnerScreenX === "number") return "gecko";
    } catch (e0) {}
    if (/Firefox\//.test(ua) || /FxiOS\//.test(ua)) return "gecko";
    if (/Gecko\//.test(ua) && !/like Gecko/.test(ua)) return "gecko";
    if (global.webkitRequestAnimationFrame || /AppleWebKit\//.test(ua)) return "webkit";
    return "unknown";
  }

  /**
   * Single-shot screen metrics. Firefox Fingerprinting Protection may alter
   * availWidth/Height and log once — cache avoids repeat reads/noise.
   * Emits screen_fp_protection_suspect when values look RFP-spoofed.
   */
  var _l1ScreenCache = null;
  function readScreenMetrics() {
    if (_l1ScreenCache) return _l1ScreenCache;
    var scr = global.screen || {};
    var w = scr.width != null ? scr.width : null;
    var h = scr.height != null ? scr.height : null;
    var aw = scr.availWidth != null ? scr.availWidth : null;
    var ah = scr.availHeight != null ? scr.availHeight : null;
    var rfp = false;
    try {
      if (w != null && h != null && aw != null && ah != null) {
        if (aw === w && ah === h) rfp = true;
        if (w === 1000 && h === 1000) rfp = true;
      }
    } catch (e) {}
    _l1ScreenCache = {
      screen_width: w,
      screen_height: h,
      screen_avail_width: aw,
      screen_avail_height: ah,
      color_depth: scr.colorDepth != null ? scr.colorDepth : null,
      pixel_depth: scr.pixelDepth != null ? scr.pixelDepth : null,
      screen_fp_protection_suspect: rfp,
    };
    return _l1ScreenCache;
  }

  function timezone() {
    return safe(function () {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    }, "");
  }

  function intlLocale() {
    return safe(function () {
      return Intl.DateTimeFormat().resolvedOptions().locale || "";
    }, "");
  }

  /** probe_dag_v2 decision stamp for B0 (design §9.3): version + engine claim
   *  + skip count. Engines are feature claims; UA weight ≈ 0 by design. */
  function dagDecisionStamp() {
    try {
      var consumed = global.__GR_DAG_V2_CONSUMED__ || null;
      if (consumed) {
        var skippedN = 0;
        Object.keys(consumed.skipped || {}).forEach(function () {
          skippedN++;
        });
        return {
          dag_v2_version: consumed.version || 0,
          dag_v2_engine: consumed.engine || "unknown",
          dag_v2_skipped_n: skippedN,
        };
      }
      if (global.GRPackLoader && typeof global.GRPackLoader.dagUploadStamp === "function") {
        return global.GRPackLoader.dagUploadStamp();
      }
    } catch (eD) {}
    return {
      dag_v2_version: 0,
      dag_v2_engine: engineFamily((global.navigator && global.navigator.userAgent) || ""),
      dag_v2_skipped_n: 0,
    };
  }

  /** B0 surface — first evidence for short-visit analyze. */
  function collectB0() {
    var nav = global.navigator || {};
    var sm = readScreenMetrics();
    var scr = { width: sm.screen_width, height: sm.screen_height };
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var dagStamp = dagDecisionStamp();
    return {
      user_agent: ua.slice(0, 500),
      language: nav.language || "",
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: platform,
      vendor: nav.vendor || "",
      os_family: deriveOsFamily(ua, platform),
      form_class: formClass(scr, nav),
      engine_family: engineFamily(ua),
      dag_v2_version: dagStamp.dag_v2_version,
      dag_v2_engine: dagStamp.dag_v2_engine,
      dag_v2_skipped_n: dagStamp.dag_v2_skipped_n,
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: sm.screen_width,
      screen_height: sm.screen_height,
      screen_avail_width: sm.screen_avail_width,
      screen_avail_height: sm.screen_avail_height,
      screen_fp_protection_suspect: sm.screen_fp_protection_suspect,
      color_depth: sm.color_depth,
      pixel_depth: sm.pixel_depth,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      cookie_enabled: !!nav.cookieEnabled,
      timezone: timezone(),
      timezone_offset_min: safe(function () {
        return new Date().getTimezoneOffset();
      }, null),
      intl_locale: intlLocale(),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      outer_width: typeof global.outerWidth !== "undefined" ? global.outerWidth : null,
      outer_height: typeof global.outerHeight !== "undefined" ? global.outerHeight : null,
      inner_width: typeof global.innerWidth !== "undefined" ? global.innerWidth : null,
      inner_height: typeof global.innerHeight !== "undefined" ? global.innerHeight : null,
      webdriver: !!nav.webdriver,
      lite: true,
      race: "l1",
      l1_batch: "B0_bootstrap",
    };
  }

  function collectB1() {
    var nav = global.navigator || {};
    var ua = nav.userAgent || "";
    var f = {
      webdriver: !!nav.webdriver,
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: nav.platform || "",
      user_agent: ua.slice(0, 500),
      automation: {
        webdriver: !!nav.webdriver,
        playwright: !!(
          (nav.webdriver && /HeadlessChrome|Playwright/i.test(ua)) ||
          global._playwright ||
          global.__playwright ||
          global.__pwInitScripts
        ),
        selenium: !!(
          global.document &&
          global.document.documentElement &&
          global.document.documentElement.getAttribute &&
          global.document.documentElement.getAttribute("webdriver")
        ),
        cdc: false,
        phantom: !!(global.callPhantom || global._phantom),
        nightmare: !!global.__nightmare,
      },
      chrome_runtime: !!(global.chrome && global.chrome.runtime),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      headless_ua: /HeadlessChrome|PhantomJS/i.test(ua),
      lite: true,
      race: "l1",
      l1_batch: "B1_conflict",
    };
    try {
      f.automation.cdc = !!(
        global.cdc_adoQpoasnfa76pfcZLmcfl_Array || global.cdc_adoQpoasnfa76pfcZLmcfl_Promise
      );
    } catch (e) {}
    return f;
  }

  function collectB12() {
    var nav = global.navigator || {};
    var scr = global.screen || {};
    return {
      webdriver: !!nav.webdriver,
      chrome_runtime: !!(global.chrome && global.chrome.runtime),
      chrome_app: !!(global.chrome && global.chrome.app),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      mime_types_length: nav.mimeTypes ? nav.mimeTypes.length : null,
      inner_width: typeof global.innerWidth !== "undefined" ? global.innerWidth : null,
      inner_height: typeof global.innerHeight !== "undefined" ? global.innerHeight : null,
      outer_width: typeof global.outerWidth !== "undefined" ? global.outerWidth : null,
      outer_height: typeof global.outerHeight !== "undefined" ? global.outerHeight : null,
      screen_width: scr.width != null ? scr.width : null,
      screen_height: scr.height != null ? scr.height : null,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      notification_permission: safe(function () {
        return typeof Notification !== "undefined" ? Notification.permission : null;
      }, null),
      lite: true,
      race: "l1",
      l1_batch: "B12_anti_camouflage",
    };
  }

  function collectB3() {
    var nav = global.navigator || {};
    var scr = global.screen || {};
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var mq = {};
    try {
      if (global.matchMedia) {
        ["(prefers-color-scheme: dark)", "(pointer: coarse)", "(hover: hover)"].forEach(function (q) {
          try {
            mq[q] = !!global.matchMedia(q).matches;
          } catch (e) {}
        });
      }
    } catch (e2) {}
    return {
      timezone: timezone(),
      timezone_offset_min: safe(function () {
        return new Date().getTimezoneOffset();
      }, null),
      language: nav.language || "",
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: platform,
      os_family: deriveOsFamily(ua, platform),
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: scr.width != null ? scr.width : null,
      screen_height: scr.height != null ? scr.height : null,
      color_depth: scr.colorDepth != null ? scr.colorDepth : null,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      media_queries_lite: mq,
      intl_locale: intlLocale(),
      lite: true,
      race: "l1",
      l1_batch: "B3_system",
    };
  }

  function collectB2() {
    var nav = global.navigator || {};
    var sm = readScreenMetrics();
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var f = {
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: sm.screen_width,
      screen_height: sm.screen_height,
      screen_avail_width: sm.screen_avail_width,
      screen_avail_height: sm.screen_avail_height,
      screen_fp_protection_suspect: sm.screen_fp_protection_suspect,
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      color_depth: sm.color_depth,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      timezone: timezone(),
      os_family: deriveOsFamily(ua, platform),
      webgl_support: false,
      webgl_vendor: "",
      webgl_renderer: "",
      webgl_unmasked_vendor: "",
      webgl_unmasked_renderer: "",
      webgl2_support: false,
      lite: true,
      race: "l1",
      l1_batch: "B2_hardware",
    };
    try {
      if (typeof document !== "undefined" && document.createElement) {
        var c = document.createElement("canvas");
        c.width = 1;
        c.height = 1;
        var gl =
          c.getContext("webgl") ||
          c.getContext("experimental-webgl") ||
          c.getContext("webgl2");
        var gl2 = null;
        try {
          gl2 = c.getContext("webgl2");
        } catch (e2a) {}
        if (gl) {
          f.webgl_support = true;
          try {
            f.webgl_vendor = gl.getParameter(gl.VENDOR) || "";
            f.webgl_renderer = gl.getParameter(gl.RENDERER) || "";
            f.webgl_version = gl.getParameter(gl.VERSION) || "";
          } catch (eV) {}
          try {
            var dbg = gl.getExtension("WEBGL_debug_renderer_info");
            if (dbg) {
              f.webgl_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) || "";
              f.webgl_unmasked_renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "";
            }
          } catch (eD) {}
          try {
            var ext = gl.getSupportedExtensions();
            f.webgl_extensions_count = ext ? ext.length : 0;
          } catch (eE) {}
          // Class-K needs texture before CIF; L1 B2 is often the only early caps path
          // (registry B2 may be deduped after this batch lands).
          try {
            var tex = null;
            try {
              tex = gl.getParameter(gl.MAX_TEXTURE_SIZE);
            } catch (eT0) {}
            // Fallback enum (0x0D33) if constant missing / governor muted pname.
            if (!(typeof tex === "number" && isFinite(tex) && tex > 0)) {
              try {
                tex = gl.getParameter(0x0d33);
              } catch (eT1) {}
            }
            if (!(typeof tex === "number" && isFinite(tex) && tex > 0) && gl2) {
              try {
                tex = gl2.getParameter(gl2.MAX_TEXTURE_SIZE || 0x0d33);
              } catch (eT2) {}
            }
            if (typeof tex === "number" && isFinite(tex) && tex > 0) {
              f.gl_max_texture_size = tex | 0;
              f.webgl_max_texture = tex | 0;
              f.b2_caps_coland = true;
            }
          } catch (eT) {}
          // Do not loseContext here — kills shared governor pool and later caps probes.
        }
        f.webgl2_support = !!gl2;
      }
    } catch (eG) {}
    return f;
  }

  var PRIORITIES = {
    B0_bootstrap: 120,
    B1_conflict: 110,
    B12_anti_camouflage: 105,
    B2_hardware: 100,
    B3_system: 98,
  };

  /** Progressive after B0: analysis-critical companions (brain main_core / thin_main). */
  var REST_ORDER = ["B1_conflict", "B12_anti_camouflage", "B2_hardware", "B3_system"];

  var COLLECTORS = {
    B0_bootstrap: collectB0,
    B1_conflict: collectB1,
    B12_anti_camouflage: collectB12,
    B2_hardware: collectB2,
    B3_system: collectB3,
  };

  function stampPageId(fields, ctx) {
    var f = fields || {};
    if (f.page_id != null) return f;
    var pid =
      (ctx && ctx.page_id) ||
      (ctx && ctx.fields && ctx.fields.page_id) ||
      global.__GR_PAGE_ID__ ||
      null;
    if (pid == null) return f;
    return Object.assign({}, f, { page_id: pid });
  }

  function enqueueBatch(ctx, batchId, fields, priority) {
    if (!ctx || !ctx.queue || typeof ctx.queue.enqueue !== "function") {
      throw new Error("l1_enqueue_needs_queue");
    }
    var prio = priority != null ? priority : PRIORITIES[batchId] || 90;
    var f = stampPageId(fields, ctx);
    ctx.queue.enqueue({
      session_id: ctx.session_id,
      batch_id: batchId,
      source: "main",
      inject_path: ctx.inject_path,
      priority: prio,
      payload: {
        fields: f,
        sandbox_kind: "main",
        lite: true,
        race: "l1",
      },
    });
    try {
      global.__GR_PACK_KICK_ORDER__ = (global.__GR_PACK_KICK_ORDER__ || []).concat([batchId]);
      global.__GR_L1_KICKED__ = global.__GR_L1_KICKED__ || {};
      global.__GR_L1_KICKED__[batchId] = Date.now();
      if (global.GRPackLoader && GRPackLoader.markKicked) {
        GRPackLoader.markKicked(batchId);
      }
    } catch (eM) {}
    return { batch_id: batchId, priority: prio, lite: true };
  }

  function runOne(ctx, batchId) {
    var fn = COLLECTORS[batchId];
    if (!fn) return Promise.resolve(null);
    return Promise.resolve()
      .then(function () {
        return fn();
      })
      .then(function (fields) {
        return enqueueBatch(ctx, batchId, fields, PRIORITIES[batchId]);
      });
  }

  /** First evidence only. */
  function runB0(ctx) {
    return runOne(ctx, "B0_bootstrap");
  }

  /**
   * Progressive rest after B0 is enqueued (analysis-critical, short-visit).
   * Parallel collect+enqueue (edge collect while B0 upload inflight) — maximizes browser cores/network.
   * Does not load registry. B2 may touch WebGL briefly.
   */
  function runProgressiveRest(ctx) {
    // Fire all rest collectors in parallel; enqueue order is priority-sorted by queue pump.
    return Promise.all(
      REST_ORDER.map(function (id) {
        return runOne(ctx, id).catch(function (e) {
          return { batch_id: id, error: String((e && e.message) || e) };
        });
      })
    ).then(function (rows) {
      return (rows || []).filter(Boolean);
    });
  }

  /** All L1 packs: B0 then rest (for harness / short-visit full lite). */
  function runAllLite(ctx) {
    return runB0(ctx).then(function (b0) {
      return runProgressiveRest(ctx).then(function (rest) {
        return { b0: b0, rest: rest, order: ["B0_bootstrap"].concat(REST_ORDER) };
      });
    });
  }

  var api = {
    collectB0: collectB0,
    collectB1: collectB1,
    collectB2: collectB2,
    collectB3: collectB3,
    collectB12: collectB12,
    enqueueBatch: enqueueBatch,
    runB0: runB0,
    runProgressiveRest: runProgressiveRest,
    runAllLite: runAllLite,
    priorities: PRIORITIES,
    restOrder: REST_ORDER.slice(),
    analysisCriticalOrder: ["B0_bootstrap"].concat(REST_ORDER),
    batchIds: Object.keys(COLLECTORS),
    /** Pure order helper for tests (no DOM). */
    progressivePlan: function () {
      return {
        first: "B0_bootstrap",
        rest: REST_ORDER.slice(),
        // B11 binds early in boot after B0; wave2 is residual heavy only.
        deferred_heavy: ["B10_hw_curves", "B7_sandbox"],
        early_rpa: ["B11_interaction"],
        requires_registry: false,
        requires_full_modules: false,
      };
    },
  };

  global.GRL1 = api;
  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
})(typeof window !== "undefined" ? window : typeof globalThis !== "undefined" ? globalThis : this);

/* ---- gr.boot.js ---- */
/**
 * green-v5 first-pack orchestrator (design: docs/10 + v57 mp.boot semantics).
 *
 * - L1 race: queue+l1 only → B0 first (no full registry), then progressive B1/B12/B2/B3
 * - Wave-2 / sandbox / mid after registry+heavy modules (overall multi-pack speed)
 * - UploadQueue: conc=2 then ramp; analyze=false; server auto-analyze
 * - Multi-tick: poll /analyses → applyRoutePlan; pagehide flush
 */
(function (global) {
  "use strict";

  var FLAG = "__GR_BOOT_STARTED__";
  if (global[FLAG]) return;
  global[FLAG] = true;

  /**
   * Strip legacy heal pollution from the address bar without navigation.
   * Old FE used ?gr_v=<long product version>; refresh keeps that query forever
   * unless we clean it. Also drop short-lived ?gr_reload= after soft heal.
   * Does NOT touch business query params.
   */
  function stripLegacyGrQueryFromUrl() {
    try {
      if (typeof location === "undefined" || typeof history === "undefined") return false;
      if (!history.replaceState) return false;
      var href = String(location.href || "");
      if (!/[?&]gr_v=/.test(href) && !/[?&]gr_reload=/.test(href)) return false;
      var hash = "";
      var base = href;
      var hi = href.indexOf("#");
      if (hi >= 0) {
        hash = href.slice(hi);
        base = href.slice(0, hi);
      }
      var cleaned = base
        .replace(/([?&])gr_v=[^&]*/g, "$1")
        .replace(/([?&])gr_reload=[^&]*/g, "$1")
        .replace(/[?&]+$/, "")
        .replace(/\?&/, "?")
        .replace(/&&+/g, "&");
      // Fix "?&foo" or trailing junk from mid-string removal
      cleaned = cleaned.replace(/\?&+/g, "?").replace(/&&+/g, "&");
      if (cleaned.charAt(cleaned.length - 1) === "?") cleaned = cleaned.slice(0, -1);
      if (cleaned === base) return false;
      history.replaceState(history.state, "", cleaned + hash);
      return true;
    } catch (eStrip) {
      return false;
    }
  }
  // ASAP: clean sticky ?gr_v= from prior heal so user address bar stays clean on refresh.
  try {
    stripLegacyGrQueryFromUrl();
  } catch (e0) {}

  /** Install silent-probe privacy guard ASAP (blocks getUserMedia/geo/clipboard from V5). */
  function ensurePrivacyGuard() {
    try {
      if (global.GRPrivacyGuard && global.GRPrivacyGuard.install) {
        global.GRPrivacyGuard.install();
        return Promise.resolve(true);
      }
    } catch (e0) {}
    // Standard B: only load hashed URL (manifest or gen-injected). Never bare fixed basename.
    return new Promise(function (resolve) {
      try {
        var href = "";
        // Inline manifest read FIRST (concatenated-bundle minification must not be
        // able to rebind manifestAssetUrl on us — see ensurePrivacyGuardSupervised).
        try {
          var __gMan =
            global.__GR_MANIFEST__ ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
            null;
          if (__gMan && __gMan.assets && __gMan.assets.privacy_guard) {
            href = String(__gMan.assets.privacy_guard);
          } else if (__gMan && __gMan.layers && __gMan.layers.privacy_guard) {
            href = String(__gMan.layers.privacy_guard);
          }
        } catch (eMan) {}
        if (!href) {
          try {
            if (typeof manifestAssetUrl === "function") {
              href = manifestAssetUrl("privacy_guard", "gr.privacy_guard.min.js");
            }
          } catch (eM) {}
        }
        if (!href) {
          try {
            href =
              typeof assetUrl === "function"
                ? assetUrl("gr.privacy_guard.js")
                : typeof withVer === "function"
                  ? withVer("gr.privacy_guard.min.js")
                  : "";
          } catch (eA) {
            href = "";
          }
        }
        // No gen/manifest yet (pin still bootstrapping) — skip load; race pack usually inlines guard.
        if (!href || !/(?:\.|\/)[a-f0-9]{8,16}\.(min\.)?js(\?|#|$)/i.test(href)) {
          resolve(false);
          return;
        }
        var s = document.createElement("script");
        s.async = true;
        s.src = href;
        s.onload = function () {
          try {
            if (global.GRPrivacyGuard) global.GRPrivacyGuard.install();
          } catch (e1) {}
          resolve(true);
        };
        s.onerror = function () {
          resolve(false);
        };
        (document.head || document.documentElement).appendChild(s);
      } catch (e2) {
        resolve(false);
      }
    });
  }
  /**
   * Guard supervisor: idempotent, multi-path retry.
   *
   * The ASAP install above can run before the pin injects __GR_MANIFEST__
   * (guard URL unknown → skipped with no retry). This supervisor:
   *  - re-checks when __GR_PIN_READY__ / manifest assets are already there
   *  - listens for gr-pin-ready (pin dispatches it after manifest + waves)
   *  - polls a few times as a last resort (heal/soft-reboot paths)
   * Observability: stamps __GR_PRIVACY_GUARD_ATTEMPT__/__GR_PRIVACY_GUARD_SRC__
   * so runtime verification can confirm install attempts happened.
   */
  function ensurePrivacyGuardSupervised() {
    try {
      if (global.GRPrivacyGuard && global.GRPrivacyGuard.install) {
        global.GRPrivacyGuard.install();
        global.__GR_PRIVACY_GUARD_P__ = Promise.resolve(true);
        return;
      }
      var href = "";
      try {
        var __gManS =
          global.__GR_MANIFEST__ ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
          null;
        if (__gManS && __gManS.assets && __gManS.assets.privacy_guard) {
          href = String(__gManS.assets.privacy_guard);
        } else if (__gManS && __gManS.layers && __gManS.layers.privacy_guard) {
          href = String(__gManS.layers.privacy_guard);
        }
      } catch (eHref0) {}
      if (!href) {
        try {
          if (typeof manifestAssetUrl === "function") {
            href = manifestAssetUrl("privacy_guard", "gr.privacy_guard.min.js");
          }
        } catch (eHref) {}
      }
      if (!href || !/(?:\.|\/)[a-f0-9]{8,16}\.(min\.)?js(\?|#|$)/i.test(href)) return;
      global.__GR_PRIVACY_GUARD_ATTEMPT__ = Date.now();
      global.__GR_PRIVACY_GUARD_SRC__ = href;
      if (global.__GR_PRIVACY_GUARD_INJECTING__) return;
      global.__GR_PRIVACY_GUARD_INJECTING__ = true;
      var done = function () {
        try {
          if (global.GRPrivacyGuard) global.GRPrivacyGuard.install();
        } catch (eI) {}
        global.__GR_PRIVACY_GUARD_INJECTING__ = false;
      };
      var s = document.createElement("script");
      s.async = true;
      s.src = href;
      s.onload = done;
      s.onerror = function () {
        global.__GR_PRIVACY_GUARD_INJECTING__ = false;
      };
      if (document.querySelector('script[src="' + href + '"]')) {
        global.__GR_PRIVACY_GUARD_INJECTING__ = false;
        return;
      }
      (document.head || document.documentElement).appendChild(s);
    } catch (eSup) {
      try {
        global.__GR_PRIVACY_GUARD_INJECTING__ = false;
      } catch (eSup2) {}
    }
  }
  // Fire immediately (BOOT_SCRIPT_DIR set below — re-call after dir ready)
  try {
    global.__GR_PRIVACY_GUARD_P__ = null;
  } catch (eP) {}

  /**
   * Page lifecycle (maximize probe):
   * - BACKGROUNDED: tab not focused / covered — KEEP probing + uploading
   * - UNLOADING (PAGE_HIDING): pagehide/beforeunload — flush hard, stop new kicks
   * Never treat visibility=hidden as permanent PAGE_HIDING (multi-window lab).
   */
  function pageUnloading() {
    try {
      return !!(global.__GR_PAGE_UNLOADING__ || global.__GR_PAGE_HIDING__);
    } catch (e) {
      return false;
    }
  }
  function markPageBackgrounded(on) {
    try {
      global.__GR_PAGE_BACKGROUNDED__ = !!on;
      if (on) {
        // Explicit: background ≠ unload
        if (!global.__GR_PAGE_UNLOADING__) global.__GR_PAGE_HIDING__ = false;
      }
    } catch (e) {}
  }
  function markPageUnloading() {
    try {
      global.__GR_PAGE_UNLOADING__ = true;
      global.__GR_PAGE_HIDING__ = true;
    } catch (e) {}
  }
  function markPageVisible() {
    try {
      global.__GR_PAGE_BACKGROUNDED__ = false;
      if (!global.__GR_PAGE_UNLOADING__) {
        global.__GR_PAGE_HIDING__ = false;
      }
    } catch (e) {}
    try {
      var Q = global.GRUploadQueue;
      if (Q && Q.markVisible) Q.markVisible();
    } catch (eQ) {}
  }

  // Capture script tag attrs SYNCHRONOUSLY — currentScript is null after async .then().
  var BOOT_SCRIPT = document.currentScript || null;
  var BOOT_SCRIPT_DIR = (function () {
    if (BOOT_SCRIPT && BOOT_SCRIPT.src) {
      try {
        return BOOT_SCRIPT.src.replace(/\/[^/]*$/, "/");
      } catch (e) {}
    }
    return "./";
  })();
  // Privacy guard right after script dir is known
  try {
    global.__GR_PRIVACY_GUARD_P__ = ensurePrivacyGuard();
  } catch (ePG) {
    global.__GR_PRIVACY_GUARD_P__ = Promise.resolve(false);
  }
  // Guard supervisor: retry on every manifest-ready path (see ensurePrivacyGuardSupervised).
  try {
    global.ensurePrivacyGuardSupervised = ensurePrivacyGuardSupervised;
    if (global.__GR_PIN_READY__ || global.__GR_MANIFEST__) {
      ensurePrivacyGuardSupervised();
    }
    global.addEventListener("gr-pin-ready", ensurePrivacyGuardSupervised, { once: true });
    var __gGuardTries = 0;
    global.__GR_PRIVACY_GUARD_TIMER__ = setInterval(function () {
      __gGuardTries += 1;
      if (global.GRPrivacyGuard || __gGuardTries >= 120) {
        try {
          clearInterval(global.__GR_PRIVACY_GUARD_TIMER__);
          global.__GR_PRIVACY_GUARD_TIMER__ = null;
        } catch (eT) {}
        return;
      }
      if (global.__GR_MANIFEST__ || (global.__GR_BOOT__ && global.__GR_BOOT__.manifest)) {
        ensurePrivacyGuardSupervised();
      }
    }, 500);
  } catch (eSup3) {}
  var BOOT_ATTR_INJECT =
    (BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-inject-path")) || "";
  var BOOT_ATTR_ENDPOINT =
    (BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-endpoint")) || "";
  var BOOT_ATTR_GW =
    (BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-gw-base")) || "";
  var BOOT_ATTR_SITE =
    (BOOT_SCRIPT && BOOT_SCRIPT.dataset && BOOT_SCRIPT.dataset.siteId) ||
    (BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-site-id")) ||
    "";

  function resolveSiteId(opts) {
    var c = cfg();
    return (
      (opts && (opts.siteId || opts.site_id)) ||
      global.__GR_SITE_ID__ ||
      c.siteId ||
      c.site_id ||
      BOOT_ATTR_SITE ||
      ""
    );
  }
  (function stickyPrimaryFromBoot() {
    var c = global.__GR_BOOT__ || {};
    var ip = BOOT_ATTR_INJECT || c.injectPath || c.inject_path || "";
    if (ip === "nginx" || ip === "cf_worker") {
      global.__GR_INJECT_PRIMARY__ = ip;
      c.injectPath = ip;
      c.inject_path = ip;
      global.__GR_BOOT__ = c;
    }
  })();

  function cfg() {
    return global.__GR_BOOT__ || {};
  }
  function embedTokenFromUrl() {
    try {
      var c = cfg();
      if (c.embed_token) return String(c.embed_token);
      // grt 可能在 pin 脚本(currentScript)、页面配置 pin_url 或任一含 grt= 的
      // script 标签上。currentScript.src 在 entry/loader 同步执行期是资产 URL
      // (无 grt)但为真值，旧实现因此短路跳过 pin_url → open 永远缺 embed_token。
      // 逐个候选源取第一个正则命中，不再短路。
      var srcs = [];
      try {
        if (typeof document !== "undefined" && document.currentScript && document.currentScript.src)
          srcs.push(String(document.currentScript.src));
      } catch (eCs) {}
      if (c.pin_url) srcs.push(String(c.pin_url));
      try {
        if (typeof document !== "undefined" && document.querySelectorAll) {
          var tags = document.querySelectorAll('script[src*="grt="]');
          for (var i = 0; i < tags.length; i++) srcs.push(String(tags[i].src || ""));
        }
      } catch (eQs) {}
      for (var j = 0; j < srcs.length; j++) {
        var m = srcs[j].match(/[?&]grt=([^&]+)/);
        if (m) return decodeURIComponent(m[1]);
      }
    } catch (eTok) {}
    return "";
  }
  function collectCookieFields() {
    var names = [];
    try {
      var raw = cfg().cookie_fields || global.__GR_COOKIE_FIELDS__ || [];
      if (typeof raw === "string") raw = raw.split(",");
      if (Array.isArray(raw)) names = raw;
    } catch (eN) {}
    var out = {};
    if (!names.length) return out;
    var jar = "";
    try {
      jar = String(document.cookie || "");
    } catch (eJ) {
      return out;
    }
    for (var i = 0; i < names.length && i < 16; i++) {
      var n = String(names[i] || "").trim();
      if (!n || n.length > 64 || !/^[a-zA-Z0-9_.-]+$/.test(n)) continue;
      var re = new RegExp(
        "(?:^|;\\s*)" + n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "=([^;]*)"
      );
      var m = jar.match(re);
      if (m && m[1]) {
        try {
          out[n] = decodeURIComponent(m[1]).slice(0, 256);
        } catch (eD) {
          out[n] = String(m[1]).slice(0, 256);
        }
      }
    }
    return out;
  }

  function scriptDir() {
    // Standard C: always flat /g5/dist/ (+ content-hash in basename via withVer).
    // Never inject /dist/v/<version>/ into the public load path.
    try {
      if (typeof distVersionRoot === "function") {
        var root = distVersionRoot();
        if (root) return root;
      }
    } catch (eVr) {}
    var base = "";
    if (BOOT_SCRIPT_DIR && BOOT_SCRIPT_DIR !== "./") {
      base = BOOT_SCRIPT_DIR;
    } else {
      var el = document.currentScript;
      if (el && el.src) {
        try {
          base = el.src.replace(/\/[^/]*$/, "/");
        } catch (e) {
          base = "./";
        }
      } else {
        base = "./";
      }
    }
    // Collapse legacy /dist/v/<ver>/[g/<gen>/] → /dist/ when present in script dir.
    try {
      if (/\/dist\/v\//.test(base)) {
        base = base.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
      }
    } catch (eRw) {}
    return base;
  }

  function loadOne(u) {
    if (!u) return Promise.resolve();
    // Belt-and-suspenders: every script load must carry SSOT version path / ?v=.
    try {
      u = withVer(u);
    } catch (eWv) {}
    // Opaque-only: withVer drops meaningful product basenames → skip load.
    if (!u || !isHashedOrOpaqueLeaf(u)) return Promise.resolve();
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = u;
      s.async = true;
      // High priority: race/entry + collectors (B10 path). Low: dense/sandbox later layers.
      try {
        if (
          u.indexOf("gr.race") >= 0 ||
          u.indexOf("gr.entry") >= 0 ||
          u.indexOf("gr.micro") >= 0 ||
          u.indexOf("l1") >= 0 ||
          u.indexOf("upload_queue") >= 0 ||
          u.indexOf("registry.static.lite") >= 0 ||
          u.indexOf("registry.static.hard") >= 0 ||
          u.indexOf("gl_governor") >= 0
        ) {
          s.fetchPriority = "high";
        } else if (
          u.indexOf("registry.dense") >= 0 ||
          u.indexOf("sandbox") >= 0 ||
          u.indexOf("deep_probe") >= 0
        ) {
          s.fetchPriority = "low";
        }
      } catch (eFp) {}
      s.onload = function () {
        resolve();
      };
      s.onerror = function () {
        reject(new Error("load " + u));
      };
      (document.head || document.documentElement).appendChild(s);
    });
  }

  /**
   * Version self-heal (layered):
   * 1) Align globals + cool/cycle state for server product_version (always).
   * 2) Prefer hot-swap: re-load micro/entry?v=server without rewriting business URL.
   * 3) Full page reload only once per serverVer (no long gr_v= polluting URL).
   * 4) If once-flag blocked but still mismatched: hot-swap again (do not stay on old FE).
   */
  function reportVersionHeal(selfV, serverVer, action, extra) {
    try {
      if (global.GROps && GROps.versionHeal) {
        GROps.versionHeal(selfV, serverVer, action, extra || {});
        return;
      }
      if (global.GROps && GROps.report) {
        GROps.report(
          "version_heal",
          "self_heal",
          Object.assign(
            { self: selfV, server: serverVer, action: action },
            extra || {}
          ),
          action === "blocked_loop" ? "warn" : "info"
        );
      }
    } catch (eR) {}
  }

  function alignVersionGlobals(serverVer) {
    serverVer = String(serverVer || "");
    if (!serverVer) return;
    adoptServerProductVersion(serverVer);
    try {
      if (global.GRStorage && typeof GRStorage.ensureProbeStateForVersion === "function") {
        GRStorage.ensureProbeStateForVersion(serverVer);
      }
    } catch (eS) {}
  }

  /**
   * Authoritative FE self version after heal.
   * CRITICAL: inject script ?v= may stay stale (cached HTML). Prefer adopted/hot-swap tag
   * so we do not loop forever reporting self=107 while entry already swapped to 110.
   */
  function markAdoptedFeVersion(serverVer) {
    serverVer = String(serverVer || "");
    if (!serverVer) return;
    alignVersionGlobals(serverVer);
    try {
      sessionStorage.setItem("gr_fe_adopted_v", serverVer);
      sessionStorage.setItem("gr_hot_swap_ok_" + serverVer, "1");
    } catch (eM) {}
  }

  function adoptedFeVersion() {
    try {
      var a = sessionStorage.getItem("gr_fe_adopted_v");
      if (a) return String(a);
    } catch (eA) {}
    return "";
  }

  function currentFeSelfVersion() {
    // Order: adopted (hot-swap/reload success) > globals > inject script ?v=
    var adopted = adoptedFeVersion();
    if (adopted) return adopted;
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (eP) {}
    try {
      var c = cfg();
      if (c && (c.version || c.product_version || c.sdk_v)) {
        return String(c.version || c.product_version || c.sdk_v);
      }
    } catch (eC) {}
    return scriptVersion() || "";
  }

  /** Re-apply seal grant/require after module re-eval (hot-swap/entry wipes closed-over state). */
  function reapplySealGrantFromGlobals() {
    try {
      var need =
        !!global.__GR_REQUIRE_SEALED__ ||
        !!(global.__GR_BOOT__ && global.__GR_BOOT__.require_sealed_ingest) ||
        !!global.__GR_SEEN_SEALED_REQUIRED__;
      var g =
        global.__GR_SEAL_GRANT__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.seal_grant) ||
        null;
      if (global.GRSeal) {
        if (need && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
        if (g && GRSeal.setGrant) GRSeal.setGrant(g);
        if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      }
    } catch (eRe) {}
  }

  /**
   * Drop sticky collector module promises so wave1/wave2 can re-download for a new
   * product_version. Without this, B10 stays on the first-registered run() forever
   * after hot-swap entry (prod v5.8.121: sessions stamped 121 but cpu_loop_algo still v1).
   */
  function resetCollectorModuleState() {
    try {
      _restModP = null;
    } catch (e0) {}
    try {
      _hardModP = null;
    } catch (e1) {}
    try {
      _midModP = null;
    } catch (e2) {}
    try {
      if (global.GRCollectors) {
        global.GRCollectors.__hardLoaded = false;
        global.GRCollectors.__midLoaded = false;
        global.GRCollectors.__liteLoaded = false;
      }
    } catch (e3) {}
    try {
      global.__GR_FORCE_HARD_RELOAD__ = 1;
    } catch (e4) {}
  }

  function distBaseUrl() {
    // Standard C: flat /g5/dist/ only (content-hash lives in basenames).
    try {
      if (typeof distVersionRoot === "function") return distVersionRoot();
    } catch (e) {}
    var api =
      String(
        global.__GR_API_BASE__ ||
          (cfg() && (cfg().apiBase || cfg().first_party_path || cfg().assetBase)) ||
          "/g5"
      ).replace(/\/$/, "") || "/g5";
    return api + "/dist/";
  }

  /**
   * Force re-fetch lite + hard collectors at ?v=serverVer and re-register B10.
   * Does not touch the business page URL.
   */
  /** Expected lite CPU algo id (must match registry stageCpu / __h.cpu_loop_algo_id). */
  var EXPECTED_CPU_LOOP_ALGO = "gr_cpu_curve_v2_multiworkload";
  var EXPECTED_CPU_LOOP_ALGO_V3 = "gr_cpu_curve_v3_multiround_median";

  function liteAlgoId() {
    try {
      if (global.__GR_CPU_LOOP_ALGO__) return String(global.__GR_CPU_LOOP_ALGO__);
      if (global.__GR_LITE_BUILD_ALGO__) return String(global.__GR_LITE_BUILD_ALGO__);
      var h = global.GRCollectors && global.GRCollectors.__h;
      if (h && h.cpu_loop_algo_id) return String(h.cpu_loop_algo_id);
    } catch (e) {}
    return "";
  }

  function liteAlgoOk() {
    var id = liteAlgoId();
    // v3 multiround median is the reliability path; v2 single-shot still accepted
    return (
      id === EXPECTED_CPU_LOOP_ALGO ||
      id === EXPECTED_CPU_LOOP_ALGO_V3 ||
      id.indexOf("gr_cpu_curve_v3_multiround") === 0
    );
  }

  function packsContentOk(serverVer) {
    serverVer = String(serverVer || "");
    if (!serverVer || !hardPacksReady() || !liteAlgoOk()) return false;
    try {
      var packsV = String(global.__GR_FE_PACKS_VERSION__ || "");
      return packsV === serverVer;
    } catch (e) {
      return false;
    }
  }

  function reloadStaticCollectors(serverVer) {
    serverVer = String(serverVer || scriptVersion() || "");
    if (!serverVer) return Promise.resolve(false);
    alignVersionGlobals(serverVer);
    resetCollectorModuleState();
    try {
      global.__GR_FE_PACKS_VERSION__ = "";
      global.__GR_CPU_LOOP_ALGO__ = "";
    } catch (eClr) {}
    var dist = distBaseUrl();
    // Always withVer + asset_gen so we never hit sticky nginx/CDN keys of old ?v=.
    function load(src) {
      src = withVer(src);
      if (!src || !isHashedOrOpaqueLeaf(src)) {
        return Promise.reject(new Error("opaque_only_skip"));
      }
      return new Promise(function (resolve, reject) {
        try {
          var s = document.createElement("script");
          s.async = true;
          s.src = src;
          s.setAttribute("data-gr-pack-v", serverVer);
          s.onload = function () {
            resolve(true);
          };
          s.onerror = function () {
            reject(new Error("pack_reload_fail " + src));
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (eL) {
          reject(eL);
        }
      });
    }
    // Standard C: manifest opaque/hashed URLs only — no meaningful collector basenames.
    var liteUrl = manifestAssetUrl("lite", "") || "";
    var hardUrl = manifestAssetUrl("hard", "") || "";
    var reloadP = load(liteUrl)
      .catch(function () {
        return true;
      })
      .then(function () {
        return load(hardUrl).catch(function () {
          return true;
        });
      })
      .then(function () {
        var ok = hardPacksReady();
        var algoOk = liteAlgoOk();
        try {
          global.__GR_FORCE_HARD_RELOAD__ = 0;
          // Only stamp packs version when lite content identity matches expected algo.
          if (ok && algoOk) {
            global.__GR_FE_PACKS_VERSION__ = serverVer;
            global.__GR_FE_CODE_VERSION__ = serverVer;
          } else {
            global.__GR_FE_PACKS_VERSION__ = "";
          }
          if (global.GRCollectors) global.GRCollectors.__hardLoaded = !!ok;
        } catch (eV) {}
        try {
          _hardModP = Promise.resolve("hard_reloaded:" + serverVer);
          _restModP = Promise.resolve("rest_reloaded:" + serverVer);
        } catch (eP) {}
        try {
          // Dedupe: one pack_reload report per page (was 4–5× per session on v150).
          var prKey = "pr:" + String(serverVer || "");
          global.__GR_PACK_RELOAD_REPORT__ = global.__GR_PACK_RELOAD_REPORT__ || Object.create(null);
          if (!global.__GR_PACK_RELOAD_REPORT__[prKey] && global.GROps && GROps.report) {
            global.__GR_PACK_RELOAD_REPORT__[prKey] = 1;
            GROps.report(
              "fe_pack_reload",
              "self_heal",
              {
                v: serverVer,
                ok: !!ok,
                hard: !!ok,
                algo_ok: !!algoOk,
                algo: liteAlgoId(),
              },
              ok && algoOk ? "info" : "warn"
            );
          }
        } catch (eO) {}
        return !!(ok && algoOk);
      })
      .catch(function (e) {
        try {
          global.__GR_FORCE_HARD_RELOAD__ = 0;
        } catch (eF) {}
        try {
          if (global.GROps && GROps.report) {
            var msg = String(e && e.message ? e.message : e).slice(0, 240);
            // Instant close / pagehide aborts in-flight pack loads — not a hard product fault.
            var unloading =
              !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_BACKGROUNDED__) ||
              (typeof document !== "undefined" && document.visibilityState === "hidden");
            var aborted =
              /abort|cancel|networkerror|failed to fetch|load failed/i.test(msg) || unloading;
            GROps.report(
              "fe_pack_reload_fail",
              "self_heal",
              {
                v: serverVer,
                err: msg,
                unloading: !!unloading,
                aborted: !!aborted,
              },
              aborted ? "warn" : "error"
            );
          }
        } catch (eO2) {}
        return false;
      });
    // iss/62: publish in-flight registry swap so ensureStaticHardModules' final
    // readiness check can await it instead of racing the lite→hard window
    // (P0 b10_register_missing: lite re-eval replaces global.GRCollectors and
    // wipes B10 until the fresh hard file re-registers it).
    try {
      global.__GR_REGISTRY_RELOAD_P__ = reloadP;
      reloadP.then(function () {
        try {
          if (global.__GR_REGISTRY_RELOAD_P__ === reloadP) {
            global.__GR_REGISTRY_RELOAD_P__ = null;
          }
        } catch (eC0) {}
      });
    } catch (ePub) {}
    return reloadP;
  }

  /**
   * Ensure in-memory B10 packs match server product_version AND lite algo content.
   * Packs stamp alone is not enough (was set from product while algo stayed v1).
   */
  function ensurePacksMatchServerVersion(serverVer) {
    serverVer = String(serverVer || "");
    if (!serverVer) return Promise.resolve(false);
    if (packsContentOk(serverVer)) {
      return Promise.resolve(true);
    }
    return reloadStaticCollectors(serverVer);
  }

  try {
    global.reloadStaticCollectors = reloadStaticCollectors;
    global.ensurePacksMatchServerVersion = ensurePacksMatchServerVersion;
  } catch (eExp) {}

  /** Hot-swap FE agents to server version without location.replace (no URL pollution). */
  function hotSwapFeVersion(serverVer) {
    serverVer = String(serverVer || "");
    if (!serverVer) return Promise.resolve(false);
    alignVersionGlobals(serverVer);
    var base =
      String(
        (global.__GR_API_BASE__ ||
          (cfg() && (cfg().apiBase || cfg().first_party_path)) ||
          "/g5")
      ).replace(/\/$/, "") || "/g5";
    function load(src) {
      return new Promise(function (resolve, reject) {
        try {
          var s = document.createElement("script");
          s.async = true;
          s.src = src;
          s.onload = function () {
            resolve(true);
          };
          s.onerror = function () {
            reject(new Error("hot_swap_load_fail"));
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (eL) {
          reject(eL);
        }
      });
    }
    // Standard C: manifest content-hash / opaque only — never invent product basenames.
    var gl =
      (typeof manifestAssetUrl === "function" &&
        manifestAssetUrl("gl_governor", "")) ||
      "";
    var race =
      (typeof manifestAssetUrl === "function" &&
        manifestAssetUrl("race", "")) ||
      "";
    var entry =
      (typeof manifestAssetUrl === "function" &&
        (manifestAssetUrl("entry", "") ||
          (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.entry_url))) ||
      "";
    return load(gl)
      .catch(function () {
        return true;
      })
      .then(function () {
        return load(race).catch(function () {
          return true;
        });
      })
      .then(function () {
        reapplySealGrantFromGlobals();
        return load(entry);
      })
      .then(function () {
        // Entry re-eval may wipe GRSeal closed-over grant — restore durable globals.
        reapplySealGrantFromGlobals();
        // Prefer NEW entry's global reload (fresh closures); fall back to local.
        var packReload =
          (typeof global.reloadStaticCollectors === "function" &&
            global.reloadStaticCollectors) ||
          reloadStaticCollectors;
        return packReload(serverVer);
      })
      .then(function (packsOk) {
        markAdoptedFeVersion(serverVer);
        try {
          sessionStorage.removeItem("gr_upgrade_once_" + serverVer);
        } catch (eSs) {}
        var ready = false;
        try {
          ready =
            !!packsOk ||
            !!(
              global.GRCollectors &&
              global.GRCollectors.get &&
              global.GRCollectors.get("B10_hw_curves")
            );
        } catch (eR) {
          ready = !!packsOk;
        }
        return ready;
      })
      .catch(function () {
        reapplySealGrantFromGlobals();
        return false;
      });
  }

  function maybeSelfHealVersion(serverVer) {
    try {
      serverVer = String(serverVer || "");
      if (!serverVer) return false;
      var selfV = currentFeSelfVersion();
      // Already on server version — still ensure B10 packs match (sticky collectors).
      if (selfV && serverVer === selfV) {
        markAdoptedFeVersion(serverVer);
        try {
          sessionStorage.removeItem("gr_upgrade_once_" + serverVer);
        } catch (eC) {}
        // Version match but address bar may still show stale ?gr_v=112 from old heal.
        try {
          stripLegacyGrQueryFromUrl();
        } catch (eSc) {}
        // P0: product_version stamp can match while in-memory B10 is still old code.
        try {
          ensurePacksMatchServerVersion(serverVer);
        } catch (ePk) {}
        return false;
      }
      if (!selfV) {
        // Inject omitted version — adopt server without reload, but pull packs for V.
        markAdoptedFeVersion(serverVer);
        try {
          ensurePacksMatchServerVersion(serverVer);
        } catch (ePk2) {}
        return false;
      }
      // Single-flight: do not stack parallel heals.
      try {
        if (global.__GR_HEAL_INFLIGHT__ === serverVer) return true;
        global.__GR_HEAL_INFLIGHT__ = serverVer;
      } catch (eI) {}

      var key = "gr_upgrade_once_" + serverVer;
      var already = false;
      var hotOk = false;
      try {
        already = sessionStorage.getItem(key) === "1";
        hotOk = sessionStorage.getItem("gr_hot_swap_ok_" + serverVer) === "1";
      } catch (eA) {}

      // Layer 1 (preferred): hot-swap entry scripts to server ?v= — no business URL rewrite.
      // Layer 2: one soft document reload with short gr_reload bust (only if hot-swap failed).
      // Layer 3: if still mismatched after reload once — hot-swap again, never blocked_loop stuck.
      if (hotOk && already) {
        // Reload already tried + prior hot-swap claimed ok but self still wrong → re-swap once.
        reportVersionHeal(selfV, serverVer, "hot_swap_retry", { reason: "still_mismatched" });
        hotSwapFeVersion(serverVer).then(function (ok) {
          try {
            global.__GR_HEAL_INFLIGHT__ = "";
          } catch (eF) {}
          if (!ok) {
            // Last resort: mark adopted so we stop thrashing ops; inject must be fixed server-side.
            markAdoptedFeVersion(serverVer);
            reportVersionHeal(selfV, serverVer, "adopt_without_swap", { reason: "hot_swap_fail" });
          }
        });
        return true;
      }

      if (!already) {
        // First mismatch: try hot-swap BEFORE full reload (fixes 107 stuck when HTML inject cached).
        reportVersionHeal(selfV, serverVer, "hot_swap", { reason: "mismatch" });
        hotSwapFeVersion(serverVer).then(function (ok) {
          try {
            global.__GR_HEAL_INFLIGHT__ = "";
          } catch (eF2) {}
          if (ok) return;
          // Hot-swap failed (network) — one soft reload.
          try {
            sessionStorage.setItem(key, "1");
          } catch (eSet) {}
          reportVersionHeal(selfV, serverVer, "reload", { reason: "hot_swap_fail" });
          try {
            // Last-resort soft reload: never re-introduce long gr_v= product version.
            // Prefer clean path; use only a short numeric bust if needed for CDN HTML.
            var u = String(location.href || "");
            var pathOnly = u
              .split("#")[0]
              .replace(/([?&])gr_v=[^&]*/g, "")
              .replace(/([?&])gr_reload=[^&]*/g, "")
              .replace(/[?&]+$/, "")
              .replace(/\?&+/g, "?")
              .replace(/&&+/g, "&");
            if (pathOnly.charAt(pathOnly.length - 1) === "?") {
              pathOnly = pathOnly.slice(0, -1);
            }
            // Avoid permanent pollution: one-shot short token only (not full version string).
            var sep = pathOnly.indexOf("?") >= 0 ? "&" : "?";
            var bust = String(Date.now() % 1e9);
            location.replace(pathOnly + sep + "gr_reload=" + bust);
          } catch (eNav) {
            try {
              location.reload();
            } catch (eRl) {}
          }
        });
        return true;
      }

      // already reloaded once: hot-swap only (no more location thrash).
      reportVersionHeal(selfV, serverVer, "hot_swap_after_once", { reason: "once_flag" });
      hotSwapFeVersion(serverVer).then(function () {
        try {
          global.__GR_HEAL_INFLIGHT__ = "";
        } catch (eF3) {}
      });
      return true;
    } catch (eH) {
      return false;
    }
  }

  function fetchSdkBootstrap(apiBase) {
    var base = String(apiBase || "/g5").replace(/\/$/, "") || "/g5";
    var q = "";
    try {
      var tok = embedTokenFromUrl();
      var sid = cfg().siteId || cfg().site_id || global.__GR_SITE_ID__ || "";
      var parts = [];
      if (tok) parts.push("grt=" + encodeURIComponent(tok));
      if (sid) parts.push("site_id=" + encodeURIComponent(String(sid)));
      if (parts.length) q = "?" + parts.join("&");
    } catch (eQ) {}
    return fetch(base + "/v1/sdk/bootstrap" + q, {
      method: "GET",
      credentials: "same-origin",
      cache: "no-store",
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        // Adopt bootstrap as manifest if nothing injected it yet (pin may have
        // taken the loader/fallback path; guard URL then stays unknown forever).
        try {
          if (j && typeof j === "object") {
            if (!global.__GR_MANIFEST__) global.__GR_MANIFEST__ = j;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            if (!global.__GR_BOOT__.manifest) global.__GR_BOOT__.manifest = j;
          }
        } catch (eBootManifest) {}
        try {
          if (j && j.require_sealed_ingest) {
            global.__GR_REQUIRE_SEALED__ = true;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.require_sealed_ingest = true;
            if (j.policy) {
              global.__GR_BOOT__.policy = Object.assign(
                {},
                global.__GR_BOOT__.policy || {},
                j.policy,
                { require_sealed_ingest: true }
              );
            }
            if (global.GRSeal && GRSeal.setRequireSealed) {
              GRSeal.setRequireSealed(true);
            }
          }
          if (j && j.cookie_fields) {
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            if (!global.__GR_BOOT__.cookie_fields) {
              global.__GR_BOOT__.cookie_fields = j.cookie_fields;
            }
            global.__GR_COOKIE_FIELDS__ = global.__GR_COOKIE_FIELDS__ || j.cookie_fields;
          }
          if (j && j.embed_token) {
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            if (!global.__GR_BOOT__.embed_token) {
              global.__GR_BOOT__.embed_token = j.embed_token;
            }
          }
        } catch (eBs) {}
        var pv = (j && (j.product_version || j.version)) || "";
        if (pv) maybeSelfHealVersion(pv);
        return j;
      })
      .catch(function () {
        return null;
      });
  }

  /**
   * SSOT product version for ALL asset URLs (?v=).
   * Priority: server bootstrap/open → global → inject boot → entry script src (LAST — can be stale).
   * Console prod evidence: preload still used ?v=v5.8.116 while inject was 123 — entry-src-first was wrong.
   */
  function scriptVersion() {
    try {
      if (global.__GR_SERVER_PRODUCT_VERSION__) {
        return String(global.__GR_SERVER_PRODUCT_VERSION__);
      }
    } catch (eS) {}
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (eP) {}
    try {
      var c = cfg();
      if (c && (c.version || c.product_version || c.sdk_v)) {
        return String(c.version || c.product_version || c.sdk_v);
      }
    } catch (eC) {}
    try {
      if (global.__GR_BOOT__ && (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) {
        return String(global.__GR_BOOT__.version || global.__GR_BOOT__.product_version);
      }
    } catch (eB) {}
    // Last resort only: current entry tag (may be stale if hot-swap left old tags).
    try {
      var src = (BOOT_SCRIPT && BOOT_SCRIPT.src) || "";
      var m = src.match(/[?&]v=([^&]+)/);
      if (m && m[1]) return decodeURIComponent(m[1]);
    } catch (eV) {}
    return "";
  }

  /**
   * Rewrite DOM link[rel=preload] / leftover script tags that still carry an old
   * ?v= or flat /g5/dist/ path onto current version path (CDN new key).
   * Console evidence: preload stayed on v5.8.116 while inject/bootstrap were already 123.
   */
  function rewriteStaleAssetQueryVersions(ver) {
    ver = String(ver || "").trim();
    if (!ver || typeof document === "undefined") return 0;
    var n = 0;
    try {
      var nodes = document.querySelectorAll(
        'link[rel="preload"][href*="/g5/"],link[rel="preload"][href*="gr."],link[rel="preload"][href*="registry."],script[src*="/g5/"],script[src*="gr."]'
      );
      for (var i = 0; i < nodes.length; i++) {
        var el = nodes[i];
        var attr = el.tagName === "LINK" ? "href" : "src";
        var cur = el.getAttribute(attr) || "";
        if (!cur) continue;
        // Only touch our FE assets
        if (!/gr\.|registry\.|pack_loader|upload_queue|collectors\//i.test(cur)) continue;
        // Loader: only rewrite ?v= (fixed name).
        if (/gr\.loader\.(min\.)?js/i.test(cur)) {
          var mL = cur.match(/[?&]v=([^&]+)/);
          var oldL = mL && mL[1] ? decodeURIComponent(mL[1]) : "";
          if (oldL && oldL !== ver) {
            var nextL = cur.replace(/([?&])v=[^&]*/, "$1v=" + encodeURIComponent(ver));
            try {
              el.setAttribute(attr, nextL);
              n++;
            } catch (eL) {}
          }
          continue;
        }
        var next = cur;
        try {
          if (typeof withVer === "function") next = withVer(cur);
        } catch (eW) {
          next = cur;
        }
        // Strip leftover ?v= on version-path assets (path is enough).
        if (/\/dist\/v\//.test(next) && /[?&]v=/.test(next)) {
          next = next
            .replace(/([?&])v=[^&]*/, "$1")
            .replace(/[?&]$/, "")
            .replace(/\?&/, "?")
            .replace(/&&/g, "&");
          if (next.charAt(next.length - 1) === "?") next = next.slice(0, -1);
        }
        if (next && next !== cur) {
          try {
            el.setAttribute(attr, next);
            n++;
          } catch (eSet) {}
        }
      }
    } catch (eRwDom) {}
    return n;
  }

  /** Stamp server version so every withVer/loadOne/preload uses the same bust token. */
  function adoptServerProductVersion(ver, assetGenToken) {
    ver = String(ver || "").trim();
    if (!ver) return "";
    try {
      global.__GR_SERVER_PRODUCT_VERSION__ = ver;
      global.__GR_PRODUCT_VERSION__ = ver;
      global.__GR_BOOT__ = global.__GR_BOOT__ || {};
      global.__GR_BOOT__.version = ver;
      global.__GR_BOOT__.product_version = ver;
      if (assetGenToken) {
        global.__GR_ASSET_GEN__ = String(assetGenToken);
        global.__GR_BOOT__.asset_gen = String(assetGenToken);
      }
    } catch (eA) {}
    // Drop sticky adopt/cool from other versions so next loads do not pin old ?v=.
    try {
      var adopted = sessionStorage.getItem("gr_fe_adopted_v") || "";
      if (adopted && adopted !== ver) {
        sessionStorage.removeItem("gr_fe_adopted_v");
        sessionStorage.removeItem("gr_hot_swap_ok_" + adopted);
        sessionStorage.removeItem("gr_upgrade_once_" + adopted);
      }
      sessionStorage.setItem("gr_fe_adopted_v", ver);
    } catch (eSs) {}
    try {
      rewriteStaleAssetQueryVersions(ver);
    } catch (eDom) {}
    return ver;
  }
  try {
    global.adoptServerProductVersion = adoptServerProductVersion;
  } catch (eExpAv) {}

  /** Optional second bust token from bootstrap (legacy query keys). */
  function assetGen() {
    try {
      if (global.__GR_ASSET_GEN__) return String(global.__GR_ASSET_GEN__);
    } catch (e0) {}
    try {
      var b = global.__GR_BOOT__ || {};
      if (b.asset_gen) return String(b.asset_gen);
    } catch (e1) {}
    return "";
  }

  /** Flat dist root: /g5/dist/ — Standard C public load base (no version segments). */
  function distFlatRoot() {
    try {
      var api = String(
        global.__GR_API_BASE__ ||
          (cfg() && (cfg().apiBase || cfg().first_party_path || cfg().assetBase)) ||
          "/g5"
      ).replace(/\/$/, "");
      return api + "/dist/";
    } catch (e) {
      return "/g5/dist/";
    }
  }

  /**
   * Asset base for loads: manifest.asset_base if set, else flat /g5/dist/.
   * Never returns /dist/v/<version>/… (version must not appear in public URLs).
   */
  function distVersionRoot() {
    try {
      var ab =
        global.__GR_ASSET_BASE__ ||
        (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_base) ||
        "";
      if (ab) {
        ab = String(ab).replace(/\/?$/, "/");
        // Collapse accidental legacy version path to flat dist.
        ab = ab.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
        return ab;
      }
    } catch (eAb) {}
    return distFlatRoot();
  }

  /**
   * True when basename is already Standard-C opaque or content-hashed:
   * - pure opaque: `a1b2c3d4e5f6.min.js`
   * - embedded: `gr.race.a1b2c3d4e5f6.min.js`
   */
  function isHashedOrOpaqueLeaf(name) {
    var base = String(name || "")
      .split("?")[0]
      .split("#")[0]
      .replace(/^.*\//, "");
    if (/^[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base)) return true;
    if (/(?:\.|\/)[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base)) return true;
    return false;
  }

  /**
   * Inject opaque content-hash / asset_gen into basename when missing (Standard C).
   * Server strip_content_hash maps any 8–16 hex token → logical on-disk name.
   * Supports .min.js / .js / .wasm / .html / .css.
   */
  function injectGenBasename(url) {
    var u = String(url || "");
    if (!u) return u;
    if (isHashedOrOpaqueLeaf(u)) return u;
    try {
      var m0 = u.match(/^(.*\/)?([^/?#]+)([?#].*)?$/);
      var name0 = m0 && m0[2] ? m0[2] : "";
      // Opaque-only: never emit meaningful product basenames on the wire
      // (even when asset_gen is not yet known).
      var ar0 =
        (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_route) ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.asset_route) ||
        "";
      if (
        (ar0 === "opaque_content_hash" ||
          ar0 === "content_hash_filename" ||
          // Default after 6.0.20: treat first-party loads as opaque-only.
          true) &&
        name0 &&
        /(?:^|[\/.])(gr\.|registry\.|pack_loader|gl_governor|probe_self_heal|origin_coordinator|nest_frame|upload_queue|fe_impl|seal_v2)/i.test(
          name0
        )
      ) {
        // Only allow when already hashed/opaque (checked above) — else drop.
        return "";
      }
    } catch (eAr0) {}
    var gen = "";
    try {
      gen = String(
        (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_gen) ||
          global.__GR_ASSET_GEN__ ||
          assetGen() ||
          ""
      );
    } catch (eG) {
      gen = "";
    }
    if (!gen) return u;
    try {
      var m = u.match(/^(.*\/)?([^/?#]+)([?#].*)?$/);
      if (!m || !m[2]) return u;
      var name = m[2];
      if (isHashedOrOpaqueLeaf(name)) return u;
      if (/\.min\.js$/i.test(name)) name = name.replace(/\.min\.js$/i, "." + gen + ".min.js");
      else if (/\.js$/i.test(name)) name = name.replace(/\.js$/i, "." + gen + ".js");
      else if (/\.wasm$/i.test(name)) name = name.replace(/\.wasm$/i, "." + gen + ".wasm");
      else if (/\.html$/i.test(name)) name = name.replace(/\.html$/i, "." + gen + ".html");
      else if (/\.css$/i.test(name)) name = name.replace(/\.css$/i, "." + gen + ".css");
      else return u;
      return (m[1] || "") + name + (m[3] || "");
    } catch (eI) {
      return u;
    }
  }

  /**
   * Standard C URL normalizer:
   * - keep content-hash basenames
   * - strip legacy /dist/v/<ver>/[g/<gen>/] → /dist/
   * - inject opaque gen into fixed basenames
   * - never emit product version in the path
   */
  function withVer(url) {
    if (!url) return url;
    var u = String(url);
    // Already content-hashed / pure-opaque basename — keep (after collapsing legacy version path).
    if (isHashedOrOpaqueLeaf(u)) {
      return u.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    }
    // Relative logical name → asset_base + hashed basename.
    if (u.indexOf("://") < 0 && u.charAt(0) !== "/" && u.indexOf("dist/") < 0) {
      var leaf = injectGenBasename(u.replace(/^\.\//, ""));
      // Opaque-only: empty leaf means do not invent a wire path.
      if (!leaf) return "";
      return distVersionRoot() + leaf;
    }
    // Collapse legacy version/gen path segments (anti-leak + unify).
    if (/\/dist\/v\/[^/]+\//.test(u)) {
      u = u.replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    }
    // Strip sticky ?v= product version query (never use version for cache bust).
    if (/[?&]v=/.test(u)) {
      u = u
        .replace(/([?&])v=[^&]*/g, "$1")
        .replace(/[?&]$/, "")
        .replace(/\?&/, "?")
        .replace(/&&/g, "&");
      if (u.charAt(u.length - 1) === "?") u = u.slice(0, -1);
    }
    // Probe API routes use fixed logical ids (r100 pack id, etc.).
    // Content-hash injection turns R46_spotcheck.js → R46_spotcheck.<gen>.js and
    // the backend rejects with invalid_pack_id (JSON 404 → MIME not executable).
    if (/\/v1\//i.test(u) || /\/r100\/pack\//i.test(u)) {
      return u;
    }
    return injectGenBasename(u);
  }

  /**
   * Resolve logical asset from pin bootstrap manifest (true per-file content hash).
   * aliasKey: assets/layers key; logicalRel: fallback path under asset_base.
   */
  function manifestAssetUrl(aliasKey, logicalRel) {
    try {
      var man =
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null;
      if (man) {
        if (aliasKey && man.assets && man.assets[aliasKey]) {
          return String(man.assets[aliasKey]);
        }
        if (aliasKey && man.layers && man.layers[aliasKey]) {
          return String(man.layers[aliasKey]);
        }
        // pack_tokens: logical collector stem → opaque URL
        var tokens =
          (man.layers && man.layers.pack_tokens) || man.pack_tokens || null;
        if (tokens && aliasKey && tokens[aliasKey]) {
          return String(tokens[aliasKey]);
        }
        // Common top-level URL fields
        if (aliasKey === "entry" && man.entry_url) return String(man.entry_url);
        if (aliasKey === "micro" && man.micro_url) return String(man.micro_url);
        if (aliasKey === "gl_governor" && man.gl_governor_url) return String(man.gl_governor_url);
        if (aliasKey === "loader" && man.loader_url) return String(man.loader_url);
        // Opaque route: do not invent meaningful filenames as fallback.
        if (
          man.asset_route === "opaque_content_hash" ||
          man.asset_route === "content_hash_filename"
        ) {
          return "";
        }
      }
    } catch (eM) {}
    // Legacy only: inject gen into logical names when not on opaque route.
    if (logicalRel) return withVer(String(logicalRel).replace(/^\.\//, ""));
    return "";
  }

  /** Prefer minified build artifacts; manifest content-hash first (Standard B). */
  function assetUrl(rel) {
    rel = String(rel || "").replace(/^\.\//, "");
    var keyMap = {
      "gr.race.min.js": "race",
      "gr.race.js": "race",
      "gr.gl_governor.min.js": "gl_governor",
      "gr.gl_governor.js": "gl_governor",
      "gr.entry.min.js": "entry",
      "gr.entry.js": "entry",
      "gr.micro.min.js": "micro",
      "gr.micro.js": "micro",
      "gr.loader.min.js": "loader",
      "gr.loader.js": "loader",
      "pack_loader.min.js": "pack_loader",
      "pack_loader.js": "pack_loader",
      "collectors/registry.static.lite.min.js": "lite",
      "collectors/registry.static.lite.js": "lite",
      "collectors/registry.static.hard.min.js": "hard",
      "collectors/registry.static.hard.js": "hard",
      "collectors/registry.mid.min.js": "mid",
      "collectors/registry.mid.js": "mid",
      "collectors/registry.dense.min.js": "dense",
      "collectors/registry.dense.js": "dense",
      "collectors/registry.b10x.min.js": "b10x",
      "collectors/registry.b10x.js": "b10x",
      "collectors/registry.random.rt.min.js": "random_rt",
      "collectors/registry.random.rt.js": "random_rt",
      "gr.privacy_guard.min.js": "privacy_guard",
      "gr.privacy_guard.js": "privacy_guard",
      "probe_self_heal.min.js": "probe_self_heal",
      "probe_self_heal.js": "probe_self_heal",
      "origin_coordinator.min.js": "origin_coordinator",
      "origin_coordinator.js": "origin_coordinator",
      "probe_lifecycle.min.js": "probe_lifecycle",
      "probe_lifecycle.js": "probe_lifecycle",
      "upload_queue.min.js": "upload_queue",
      "upload_queue.js": "upload_queue",
      "storage.min.js": "storage",
      "storage.js": "storage",
      "rpa_monitor.min.js": "rpa_monitor",
      "rpa_monitor.js": "rpa_monitor",
      "sandbox_tree.min.js": "sandbox_tree",
      "sandbox_tree.js": "sandbox_tree",
      "collectors/deep_probe_lists.min.js": "deep_probe_lists",
      "collectors/deep_probe_lists.js": "deep_probe_lists",
      "deep_probe_lists.min.js": "deep_probe_lists",
      "deep_probe_lists.js": "deep_probe_lists",
    };
    var min = /\.min\.js$/i.test(rel) ? rel : rel.replace(/\.js$/i, ".min.js");
    var alias = keyMap[min] || keyMap[rel] || "";
    if (alias) {
      var fromMan = manifestAssetUrl(alias, "");
      if (fromMan) return fromMan;
    }
    // pack_tokens stem from collectors/<stem>.min.js when keyMap missed
    try {
      var stemM = String(min).match(/(?:^|\/)([^/]+?)(?:\.min)?\.js$/i);
      if (stemM && stemM[1]) {
        var fromTok = manifestAssetUrl(stemM[1], "");
        if (fromTok) return fromTok;
      }
    } catch (eStem) {}
    return withVer(min);
  }
  function assetUrlWithFallback(rel) {
    var minRel = String(rel || "").replace(/\.js$/i, ".min.js");
    var min = assetUrl(minRel);
    // Only try non-min if min path is gen-injected (never bare fixed basename).
    var raw = withVer(String(rel || "").replace(/^\.\//, ""));
    return loadOne(min).catch(function () {
      if (raw && raw !== min) return loadOne(raw);
      throw new Error("asset_load_fail " + minRel);
    });
  }

  /** Sequential (dependency-ordered). */
  function loadSeq(urls) {
    var chain = Promise.resolve();
    urls.forEach(function (u) {
      chain = chain.then(function () {
        return loadOne(u);
      });
    });
    return chain;
  }

  /** Parallel race (v57): independent modules load concurrently. */
  function loadParallel(urls) {
    return Promise.all(
      (urls || []).filter(Boolean).map(function (u) {
        return loadOne(u).catch(function (e) {
          try { if (global.GROps) GROps.hardLoadFail(u, e && e.message); } catch (eOp) {}
        });
      })
    );
  }

  /**
   * Early <link rel=preload> for next-layer scripts (HTTP/2 push alternative).
   * Does not execute — only warms cache before ensureRestModules.
   */
  function preloadScript(relPath) {
    try {
      if (typeof document === "undefined" || !document.head) return;
      // Prefer manifest opaque URL for known logical paths (anti-leak).
      var href = "";
      try {
        href = assetUrl(String(relPath || "").replace(/^\.\//, "")) || "";
      } catch (eA) {
        href = "";
      }
      if (!href) {
        href = withVer(scriptDir() + relPath);
      }
      if (!href || !isHashedOrOpaqueLeaf(href)) return;
      if (document.querySelector('link[rel="preload"][href="' + href + '"]')) return;
      var l = document.createElement("link");
      l.rel = "preload";
      l.as = "script";
      l.href = href;
      document.head.appendChild(l);
    } catch (e0) {}
  }
  function preloadCriticalLayers() {
    // GL governor before hard/WebGL packs — prevents "Too many active WebGL contexts"
    // which stalls B10_hw_curves in headless + multi-pack runs.
    // Also warm lite+hard early so wave2 B10 does not wait on cold CDN fetch.
    // Protocol 2: prefer absolute URLs from pin manifest (always current VERSION path).
    try {
      var man =
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null;
      if (man) {
        var pre = man.preload || [];
        for (var pi = 0; pi < pre.length; pi++) {
          try {
            var u = String(pre[pi] || "");
            if (!u || typeof document === "undefined" || !document.head) continue;
            if (document.querySelector('link[rel="preload"][href="' + u + '"]')) continue;
            var lp = document.createElement("link");
            lp.rel = "preload";
            lp.as = "script";
            lp.href = u;
            document.head.appendChild(lp);
          } catch (ePre) {}
        }
        if (man.layers && man.layers.hard) {
          try {
            var hu = String(man.layers.hard);
            if (hu && !document.querySelector('link[rel="preload"][href="' + hu + '"]')) {
              var lh = document.createElement("link");
              lh.rel = "preload";
              lh.as = "script";
              lh.href = hu;
              document.head.appendChild(lh);
            }
          } catch (eH) {}
        }
        return;
      }
    } catch (eMan) {}
    // Opaque-only preload from manifest — never fall back to product basenames.
    try {
      var g0 = manifestAssetUrl("gl_governor", "");
      var l0 = manifestAssetUrl("lite", "");
      var h0 = manifestAssetUrl("hard", "");
      var p0 = manifestAssetUrl("pack_loader", "");
      function preloadAbs(href) {
        try {
          if (!href || !isHashedOrOpaqueLeaf(href)) return;
          if (document.querySelector('link[rel="preload"][href="' + href + '"]')) return;
          var l = document.createElement("link");
          l.rel = "preload";
          l.as = "script";
          l.href = href;
          document.head.appendChild(l);
        } catch (ePl) {}
      }
      preloadAbs(g0);
      preloadAbs(l0);
      preloadAbs(h0);
      preloadAbs(p0);
    } catch (ePreFb) {}
  }

  /** Ensure WebGL context pool is installed before any curve pack runs. */
  var _glGovP = null;
  function ensureGlGovernor() {
    if (global.__GR_GL_GOV_INSTALLED__) return Promise.resolve("gl_present");
    if (_glGovP) return _glGovP;
    // Manifest opaque URL only — never invent gr.gl_governor.* basenames.
    var glUrl = manifestAssetUrl("gl_governor", "");
    if (!glUrl) {
      _glGovP = Promise.resolve("gl_skip_opaque");
      return _glGovP;
    }
    _glGovP = loadOne(glUrl)
      .then(function () {
        return global.__GR_GL_GOV_INSTALLED__ ? "gl_loaded" : "gl_missing";
      })
      .catch(function () {
        return "gl_fail";
      });
    return _glGovP;
  }
  /**
   * Optional Service Worker for versioned static collectors.
   * Enable: data-enable-sw="1" on boot script, or window.__GR_ENABLE_SW__=1, or GR_ENABLE_SW via boot cfg.
   */
  function maybeRegisterServiceWorker() {
    try {
      if (!("serviceWorker" in navigator)) return;
      var c = cfg();
      var en =
        (BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-enable-sw") === "1") ||
        !!global.__GR_ENABLE_SW__ ||
        c.enable_sw === true ||
        c.enableSw === true;
      if (!en) return;
      var swUrl = withVer(scriptDir() + "gr.sw.js");
      navigator.serviceWorker.register(swUrl, { scope: scriptDir() }).then(
        function () {
          try {
            global.__GR_SW_REGISTERED__ = true;
          } catch (e0) {}
        },
        function () {}
      );
    } catch (e1) {}
  }
  /** After wave2 idle: warm mid.core only (not full mid/dense/R). */
  function idleWarmMidCore() {
    try {
      var run = function () {
        try {
          if (global.GRCollectors && global.GRCollectors.__midFamilies && global.GRCollectors.__midFamilies.core)
            return;
          loadMidFamily("core").catch(function () {});
        } catch (e1) {}
      };
      if (typeof requestIdleCallback === "function") {
        requestIdleCallback(function () {
          run();
        }, { timeout: 4000 });
      } else {
        setTimeout(run, 2500);
      }
    } catch (e2) {}
  }

  /**
   * L1 race modules only — queue + L1 lite (NO full registry).
   * Prefer single race pack (gr.race.min.js) to minimize RTT (v57 mp.race).
   */
  var _raceModP = null;
  var _restModP = null;
  var _fullModP = null;
  function ensureRaceModules() {
    if (_raceModP) return _raceModP;
    var base = scriptDir();
    // Warm next layer while race downloads
    preloadCriticalLayers();
    maybeRegisterServiceWorker();

    /**
     * Gap supervisor is in race pack, but pin loads **entry** (upload_queue+l1 inlined)
     * without probe_self_heal. The old "inlined" short-circuit skipped race load AND
     * never fetched self-heal → __GR_SELF_HEAL_ACTIVE__ forever false in production.
     */
    function ensureSelfHealModule() {
      function loadHeal() {
        if (global.GRProbeSelfHeal && GRProbeSelfHeal.__ready) {
          return Promise.resolve("heal_present");
        }
        return assetUrlWithFallback("probe_self_heal.js")
          .then(function () {
            return global.GRProbeSelfHeal ? "heal_loaded" : "heal_missing";
          })
          .catch(function (e) {
            try {
              if (global.GROps && GROps.report) {
                GROps.report(
                  "self_heal_load_fail",
                  "boot",
                  { err: String(e && e.message ? e.message : e) },
                  "warn"
                );
              }
            } catch (eOp) {}
            return "heal_fail";
          });
      }
      // Origin coordinator before self-heal (GPI multi-tab).
      var coordP = Promise.resolve("coord_skip");
      if (!(global.GROriginCoordinator && GROriginCoordinator.__ready)) {
        coordP = assetUrlWithFallback("origin_coordinator.js")
          .then(function () {
            return global.GROriginCoordinator ? "coord_loaded" : "coord_missing";
          })
          .catch(function () {
            return "coord_fail";
          });
      } else {
        coordP = Promise.resolve("coord_present");
      }
      return coordP.then(function () {
        return loadHeal();
      });
    }

    function afterRaceReady(tag) {
      return ensureSelfHealModule()
        .then(function (healTag) {
          try {
            global.__GR_RACE_LOAD_TAG__ = String(tag || "") + "|" + String(healTag || "");
          } catch (eT) {}
          // Start supervisor ASAP if boot open has not yet (idempotent).
          try {
            if (
              global.GRProbeSelfHeal &&
              GRProbeSelfHeal.start &&
              !global.__GR_SELF_HEAL_ACTIVE__
            ) {
              GRProbeSelfHeal.start({
                need_hard_anchor: true,
                missing: ["B10_hw_curves", "B0_bootstrap"],
              });
            }
          } catch (eStart) {}
          return ensureGlGovernor().then(function () {
            return tag;
          });
        });
    }

    // Entry already ships queue+l1 (not full race). Still must load self-heal.
    if (global.GRUploadQueue && global.GRL1) {
      _raceModP = afterRaceReady("inlined");
      return _raceModP;
    }
    // Standard B: race from manifest content-hash URL (never bare gr.race.min.js).
    var raceUrl = manifestAssetUrl("race", "gr.race.min.js") || assetUrl("gr.race.min.js");
    function loadSplit() {
      var jobs = [];
      if (!global.GRPrivacyGuard) jobs.push(assetUrlWithFallback("gr.privacy_guard.js"));
      if (!global.GRProbeLifecycle) jobs.push(assetUrlWithFallback("probe_lifecycle.js"));
      if (!global.GRUploadQueue) jobs.push(assetUrlWithFallback("upload_queue.js"));
      if (!global.GRL1) jobs.push(assetUrlWithFallback("collectors/l1.js"));
      if (!global.GRStorage) jobs.push(assetUrlWithFallback("storage.js"));
      // Self-heal also ensured in afterRaceReady — include here for split path.
      if (!global.GRProbeSelfHeal) jobs.push(assetUrlWithFallback("probe_self_heal.js"));
      return Promise.all(jobs);
    }
    // Start GL governor in parallel with race pack (needed before B10/WebGL).
    ensureGlGovernor();
    _raceModP = loadOne(raceUrl)
      .then(function () {
        if (global.GRUploadQueue && global.GRL1) return "race";
        return loadSplit().then(function () {
          return "split";
        });
      })
      .catch(function () {
        return loadSplit().then(function () {
          return "split_fallback";
        });
      })
      .then(function (tag) {
        return afterRaceReady(tag);
      });
    return _raceModP;
  }
  /**
   * Rest after L1: pack_loader + **static.lite** (wave1) + rpa.
   * B10/B7 live in static.hard — loaded for wave2 via ensureStaticHardModules.
   */
  var _midModP = null;
  var _hardModP = null;
  function ensureRestModules() {
    if (_restModP) return _restModP;
    _restModP = ensureRaceModules().then(function () {
      var jobs = [];
      // pack_loader is critical for wave2 kickAll — script onload can race without defining
      // GRPackLoader in some lab/CDP stacks; verify and fetch+eval fallback.
      if (!global.GRPackLoader) {
        jobs.push(
          assetUrlWithFallback("pack_loader.js")
            .catch(function () {
              return null;
            })
            .then(function () {
              if (global.GRPackLoader) return "pack_loader_ok";
              var url =
                manifestAssetUrl("pack_loader", "pack_loader.min.js") ||
                assetUrl("pack_loader.min.js");
              return fetch(url, { credentials: "same-origin", cache: "no-store" })
                .then(function (r) {
                  if (!r || !r.ok) throw new Error("pack_loader_fetch_" + (r && r.status));
                  return r.text();
                })
                .then(function (code) {
                  // Indirect eval so assignment lands on window global.
                  (0, eval)(String(code || ""));
                  if (!global.GRPackLoader) throw new Error("pack_loader_no_global");
                  return "pack_loader_eval_ok";
                })
                .catch(function (e) {
                  try {
                    if (global.GROps) GROps.hardLoadFail(url, String((e && e.message) || e));
                  } catch (eOp) {}
                  return "pack_loader_fail";
                });
            })
        );
      }
      if (rpaEnabledForPlan() && !global.GRRpaMonitor) jobs.push(assetUrlWithFallback("rpa_monitor.js"));
      if (!global.GRCollectors) {
        // Prefer lite static (manifest hashed); fall back gen-injected paths only.
        jobs.push(
          loadOne(manifestAssetUrl("lite", "collectors/registry.static.lite.min.js") || assetUrl("collectors/registry.static.lite.min.js"))
            .catch(function () {
              return loadOne(withVer("collectors/registry.static.lite.js"));
            })
            .catch(function () {
              return loadOne(withVer("collectors/registry.static.min.js"));
            })
            .catch(function () {
              return assetUrlWithFallback("collectors/registry.static.js");
            })
            .catch(function () {
              return assetUrlWithFallback("collectors/registry.js");
            })
        );
      }
      return Promise.all(jobs);
    });
    return _restModP;
  }

  /** True only when B10 pack is actually registered (not a sticky false __hardLoaded). */
  function hardPacksReady() {
    try {
      return !!(
        global.GRCollectors &&
        global.GRCollectors.get &&
        global.GRCollectors.get("B10_hw_curves") &&
        typeof global.GRCollectors.get("B10_hw_curves").run === "function"
      );
    } catch (e) {
      return false;
    }
  }
  function hardPacksFullReady() {
    try {
      var c = global.GRCollectors;
      return !!(
        hardPacksReady() &&
        c.get("B7_sandbox") &&
        typeof c.get("B7_sandbox").run === "function"
      );
    } catch (e2) {
      return hardPacksReady();
    }
  }
  /** Wave2: B10 + B7 definitions (helpers already on GRCollectors.__h from lite). */
  function ensureStaticHardModules() {
    // If a prior attempt marked loaded without B10, allow retry.
    if (_hardModP && hardPacksReady()) return _hardModP;
    if (_hardModP && !hardPacksReady()) {
      try {
        if (global.GRCollectors) global.GRCollectors.__hardLoaded = false;
      } catch (eClr) {}
      _hardModP = null;
    }
    function loadHardOnce() {
      // Standard B: manifest hard URL first, then gen-injected candidates only.
      var urls = [
        manifestAssetUrl("hard", "collectors/registry.static.hard.min.js"),
        assetUrl("collectors/registry.static.hard.min.js"),
        withVer("collectors/registry.static.hard.js"),
        withVer("collectors/registry.static.min.js"),
        withVer("collectors/registry.static.js"),
      ].filter(Boolean);
      var i = 0;
      function next() {
        if (hardPacksReady()) return Promise.resolve("hard_present");
        if (i >= urls.length) return Promise.resolve("hard_exhausted");
        var u = urls[i++];
        return loadOne(u)
          .then(function () {
            if (hardPacksReady()) return "hard_loaded:" + u;
            return next();
          })
          .catch(function () {
            return next();
          });
      }
      return next();
    }
    function loadB10xDomain() {
      // Domain deepen packs (B10x_*) — after multipath helpers exist on collectors.
      var urls = [
        manifestAssetUrl("b10x", "collectors/registry.b10x.min.js"),
        assetUrl("collectors/registry.b10x.min.js"),
        withVer("collectors/registry.b10x.js"),
      ].filter(Boolean);
      var i = 0;
      function next() {
        if (i >= urls.length) return Promise.resolve("b10x_skip");
        var u = urls[i++];
        return loadOne(u)
          .then(function () {
            return "b10x_loaded:" + u;
          })
          .catch(function () {
            return next();
          });
      }
      return next();
    }
    _hardModP = ensureRestModules()
      .then(function () {
        return ensureGlGovernor();
      })
      .then(function () {
        if (hardPacksReady()) {
          try {
            if (global.GRCollectors) global.GRCollectors.__hardLoaded = true;
          } catch (e0) {}
          return "hard_present";
        }
        return loadHardOnce();
      })
      .then(function (tag) {
        return loadB10xDomain().then(function () {
          return tag;
        });
      })
      .then(function (tag) {
        function stampAndReport(ok, extra) {
          try {
            if (global.GRCollectors) global.GRCollectors.__hardLoaded = !!ok;
          } catch (e1) {}
          // Content-proven stamp only (liteAlgoOk after lite script eval).
          try {
            if (ok && liteAlgoOk()) {
              var pv =
                scriptVersion() ||
                (global.__GR_SERVER_PRODUCT_VERSION__ ||
                  global.__GR_PRODUCT_VERSION__ ||
                  (global.__GR_BOOT__ &&
                    (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
                  "");
              if (pv) {
                global.__GR_FE_PACKS_VERSION__ = String(pv);
                global.__GR_FE_CODE_VERSION__ = String(pv);
              }
            }
          } catch (ePv) {}
          if (!ok) {
            // Allow a later plan-driven retry (do not sticky-fail forever).
            _hardModP = null;
            try {
              if (global.GROps) {
                var payload = {
                  tag: String(tag || ""),
                  engine: (function () {
                    try {
                      return (global.GROps && GROps.engineFamily) || "";
                    } catch (eE) {
                      return "";
                    }
                  })(),
                };
                if (extra) {
                  for (var ek in extra) {
                    if (Object.prototype.hasOwnProperty.call(extra, ek)) payload[ek] = extra[ek];
                  }
                }
                GROps.report("b10_register_missing", "load_pack", payload, "error");
              }
            } catch (eW) {}
            return "hard_missing_b10";
          }
          return tag || "hard_loaded";
        }
        if (hardPacksReady()) return stampAndReport(true, null);
        // iss/62 P0 (b10_register_missing): a version self-heal reload swaps
        // global.GRCollectors (fresh lite eval) and re-registers B10 async.
        // Await the in-flight swap, then one direct hard re-load, before
        // declaring B10 missing — closes the false-positive race window and
        // gives a real second chance when the reload's hard leg failed.
        var reloadP = null;
        try {
          reloadP = global.__GR_REGISTRY_RELOAD_P__ || null;
        } catch (eR0) {}
        var awaited = reloadP
          ? Promise.race([
              Promise.resolve(reloadP).catch(function () {
                return false;
              }),
              new Promise(function (r) {
                setTimeout(function () {
                  r("reload_wait_timeout");
                }, 4000);
              }),
            ])
          : Promise.resolve("no_reload_in_flight");
        return awaited.then(function () {
          if (hardPacksReady()) return stampAndReport(true, null);
          return loadHardOnce().then(function () {
            return stampAndReport(hardPacksReady(), {
              reload_awaited: reloadP ? 1 : 0,
              retried_load: 1,
            });
          });
        });
      });
    return _hardModP;
  }

  /** Mid family ids present in a plan: core | gpu | misc */
  function midFamiliesForPlan(routePlan) {
    var packs = (routePlan && routePlan.packs) || [];
    var fam = { core: false, gpu: false, misc: false };
    var GPU = {
      B17_hw_physical: 1,
      B18_webgpu: 1,
      B19_eme_media: 1,
      B20_challenge_seed: 1,
      B22_gpu_timer: 1,
      B23_native_canvas_hedge: 1,
      B30_gpu_bandwidth: 1,
      B31_shader_numeric: 1,
      B33_caps_pressure: 1,
      B34_cpu_cache_ladder: 1,
      B36_raster_msaa: 1,
      B37_thermal_drift_lite: 1,
      B42_thermal_drift_full: 1,
      B46_audio_deep: 1,
      B47_sab_clock: 1,
    };
    var CORE = {
      B4_mobile: 1,
      B5_census: 1,
      B6_risk: 1,
      B9_network: 1,
      B13_authorized: 1,
      B14_css_protocol: 1,
      B15_cross_curves: 1,
      B16_fast_signals: 1,
      B21_census_volume: 1,
      B24_material_crosscheck: 1,
      B25_clock_raf: 1,
      B26_agent_parity: 1,
      B27_storage_privacy: 1,
      B28_permissions_media: 1,
      B29_sensors_battery: 1,
    };
    for (var i = 0; i < packs.length; i++) {
      var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
      if (GPU[id]) fam.gpu = true;
      else if (CORE[id]) fam.core = true;
      else {
        var m = id.match(/^B(\d+)/);
        if (m) {
          var n = parseInt(m[1], 10);
          if (n >= 4 && n <= 46 && n !== 7 && n !== 8 && n !== 10 && n !== 11 && n !== 12) {
            if (GPU[id]) fam.gpu = true;
            else if (CORE[id]) fam.core = true;
            else fam.misc = true;
          }
        }
      }
    }
    var out = [];
    if (fam.core) out.push("core");
    if (fam.gpu) out.push("gpu");
    if (fam.misc) out.push("misc");
    return out;
  }

  function loadMidFamily(family) {
    var base = scriptDir();
    var min = "collectors/registry.mid." + family + ".min.js";
    var raw = "collectors/registry.mid." + family + ".js";
    return loadOne(withVer(base + min))
      .catch(function () {
        return loadOne(withVer(base + raw));
      })
      .then(function () {
        return "mid_" + family;
      });
  }

  /**
   * Mid/dynamic packs — load only families required by plan (core/gpu/misc).
   * Fallback: full registry.mid.min.js if family files missing.
   */
  function ensureMidModules(routePlan) {
    var families = routePlan ? midFamiliesForPlan(routePlan) : ["core", "gpu", "misc"];
    if (!families.length) families = ["core", "gpu", "misc"];
    return ensureRestModules().then(function () {
      try {
        var C0 = global.GRCollectors;
        if (C0 && C0.__midLoaded) return "mid_all_present";
        // Skip families already registered (per-family flags or sample packs).
        if (C0) {
          var mf = C0.__midFamilies || {};
          families = families.filter(function (f) {
            if (mf[f]) return false;
            if (f === "core" && C0.get && C0.get("B16_fast_signals")) return false;
            if (f === "gpu" && C0.get && C0.get("B22_gpu_timer")) return false;
            if (f === "misc" && C0.get && C0.get("B35_dom_perf")) return false;
            return true;
          });
          if (!families.length) return "mid_families_present";
        }
      } catch (e0) {}
      var base = scriptDir();
      // Try family files first
      return Promise.all(
        families.map(function (f) {
          return loadMidFamily(f).catch(function () {
            return null;
          });
        })
      ).then(function (st) {
        var any = st.some(function (s) {
          return s;
        });
        if (any) return st;
        // Fallback full mid
        return loadOne(withVer(base + "collectors/registry.mid.min.js"))
          .catch(function () {
            return loadOne(withVer(base + "collectors/registry.mid.js"));
          })
          .catch(function () {
            return assetUrlWithFallback("collectors/registry.js");
          })
          .then(function () {
            try {
              if (global.GRCollectors) global.GRCollectors.__midLoaded = true;
            } catch (e1) {}
            return "mid_full_fallback";
          });
      });
    });
  }

  /** True if route_plan asks for dense B47–B80 packs. */
  function planNeedsDense(routePlan) {
    var packs = (routePlan && routePlan.packs) || [];
    for (var i = 0; i < packs.length; i++) {
      var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
      var m = id.match(/^B(\d+)/);
      if (m && parseInt(m[1], 10) >= 47 && parseInt(m[1], 10) <= 80) return true;
    }
    return false;
  }
  function planNeedsMid(routePlan) {
    return midFamiliesForPlan(routePlan).length > 0;
  }
  function planNeedsHardStatic(routePlan) {
    var packs = (routePlan && routePlan.packs) || [];
    for (var i = 0; i < packs.length; i++) {
      var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
      if (
        id === "B10_hw_curves" ||
        id.indexOf("B10x_") === 0 ||
        id === "B7_sandbox" ||
        id === "mid.curves"
      )
        return true;
    }
    return false;
  }
  /**
   * Load only collector layers required by route_plan.
   * lite already in rest; hard/mid-families/dense/R on demand.
   */
  function ensureModulesForRoutePlan(routePlan) {
    var chain = ensureRestModules();
    if (planNeedsHardStatic(routePlan)) {
      chain = chain.then(function () {
        return ensureStaticHardModules();
      });
    }
    if (planNeedsMid(routePlan)) {
      chain = chain.then(function () {
        return ensureMidModules(routePlan);
      });
    }
    if (planNeedsDense(routePlan)) {
      chain = chain.then(function () {
        return ensureDenseModules();
      });
    }
    if (typeof global.ensureRandomVerifyPacksForPlan === "function") {
      chain = chain.then(function () {
        return global.ensureRandomVerifyPacksForPlan(routePlan);
      });
    }
    return chain;
  }
  global.ensureModulesForRoutePlan = ensureModulesForRoutePlan;
  global.ensureMidModules = ensureMidModules;
  global.ensureDenseModules = ensureDenseModules;
  global.ensureStaticHardModules = ensureStaticHardModules;
  /** Dense B47–B79 packs (claim-obs / family digests) — after mid + probe lists. */
  var _denseModP = null;
  function ensureDenseModules() {
    if (_denseModP) return _denseModP;
    _denseModP = Promise.resolve()
      .then(function () {
        if (global.GRCollectors && global.GRCollectors.__denseLoaded) {
          return "dense_inlined";
        }
        if (
          global.GRCollectors &&
          global.GRCollectors.get &&
          global.GRCollectors.get("B47_api_flags_detail")
        ) {
          global.GRCollectors.__denseLoaded = true;
          return "dense_present";
        }
        var jobs = [];
        if (!global.GRProbeLists) {
          jobs.push(assetUrlWithFallback("collectors/deep_probe_lists.js"));
        }
        var base = scriptDir();
        jobs.push(
          loadOne(withVer(base + "collectors/registry.dense.min.js")).catch(function () {
            return loadOne(withVer(base + "collectors/registry.dense.js"));
          })
        );
        return Promise.all(jobs).then(function () {
          if (global.GRCollectors) global.GRCollectors.__denseLoaded = true;
          return "dense_loaded";
        });
      })
      .catch(function (e) {
        try {
          try { if (global.GROps) GROps.hardLoadFail("dense", e && e.message); } catch (eOp) {}
        } catch (e0) {}
        return "dense_skip";
      });
    return _denseModP;
  }
  /**
   * Wave2 / sandbox path: rpa + sandbox + probe lists.
   * Mid/dense/R are NOT bulk-preloaded here — loaded by ensureModulesForRoutePlan when brain asks.
   * (R is on-demand per pack; dense only when B47+ scheduled.)
   */
  function ensureFullModules() {
    if (_fullModP) return _fullModP;
    _fullModP = ensureRestModules()
      .then(function () {
        var jobs = [];
        if (rpaEnabledForPlan() && !global.GRRpaMonitor) jobs.push(assetUrlWithFallback("rpa_monitor.js"));
        if (!global.GRProbeLists) jobs.push(assetUrlWithFallback("collectors/deep_probe_lists.js"));
        if (!global.GRSandbox) jobs.push(assetUrlWithFallback("sandbox_tree.js"));
        // Mid optional warmup after idle: do not block wave2 B10/B7 on mid+dense download.
        return Promise.all(jobs).then(function (r) {
          idleWarmMidCore();
          return r;
        });
      });
    return _fullModP;
  }
  /**
   * R100 authenticity lane (aligned with B progressive plan load / Standard C):
   *   1) static once: assets.random_rt (opaque) — runtime helpers, no 100 packs
   *   2) on route_plan:
   *        a) pack_tokens["random/Rxx_spotcheck"] opaque under /g5/dist/ (preferred)
   *        b) GET /g5/v1/r100/pack/Rxx_spotcheck.js  (fixed pack id — NO content-hash gen)
   *   3) static fallback: pack_tokens / collectors/random opaque if present
   *
   * R packs are progressive after route_plan — they must NOT block first-party
   * open / B0–B8 / B10 primary residual load.
   */
  var _randVerifyModP = null;
  var _randPackP = Object.create(null);
  /** First-party API base for pseudo-static R templates (same origin as open/ingest). */
  function resolveApiBaseForR100() {
    try {
      if (global.__GR_API_BASE__) return String(global.__GR_API_BASE__).replace(/\/$/, "");
    } catch (e0) {}
    try {
      var c = cfg();
      var ep =
        BOOT_ATTR_ENDPOINT ||
        c.apiBase ||
        c.api_base ||
        c.endpoint ||
        global.__GR_FIRST_PARTY__ ||
        "";
      if (ep && ep !== true && ep !== 1) return String(ep).replace(/\/$/, "");
    } catch (e1) {}
    // Same-origin relative under inject path (lab /g5)
    try {
      if (global.__GR_BOOT__ && global.__GR_BOOT__.first_party_path) {
        return String(global.__GR_BOOT__.first_party_path).replace(/\/$/, "");
      }
    } catch (e2) {}
    return "/g5";
  }
  function ensureRandomVerifyModules() {
    if (_randVerifyModP) return _randVerifyModP;
    _randVerifyModP = Promise.resolve()
      .then(function () {
        if (global.GRCollectors && global.GRCollectors.__randomVerifyRuntime) {
          return "rand_rt_present";
        }
        if (global.GRRandomVerify && typeof global.GRRandomVerify.registerPack === "function") {
          if (global.GRCollectors) global.GRCollectors.__randomVerifyRuntime = true;
          return "rand_rt_api";
        }
        var jobs = [];
        if (!global.GRProbeLists) {
          jobs.push(assetUrlWithFallback("collectors/deep_probe_lists.js"));
        }
        // Standard C: manifest assets.random_rt only — never invent registry.* wire names.
        var rtUrl = assetUrl("collectors/registry.random.rt.min.js");
        if (rtUrl) {
          jobs.push(loadOne(rtUrl));
        } else {
          jobs.push(Promise.reject(new Error("random_rt_opaque_missing")));
        }
        return Promise.all(jobs).then(function () {
          if (global.GRCollectors) {
            global.GRCollectors.__randomVerifyRuntime = true;
            global.GRCollectors.__randomVerifyLoaded = false;
          }
          try {
            global.__GR_RANDOM_LOAD__ = "rand_rt_opaque";
          } catch (eL) {}
          return "rand_rt_loaded";
        });
      })
      .catch(function (e) {
        try {
          try { if (global.GROps) GROps.hardLoadFail("random_rt", e && e.message); } catch (eOp) {}
        } catch (e0) {}
        // Soft-fail: R packs may still register if runtime already partial.
        try {
          if (global.GRCollectors) {
            global.GRCollectors.__randomVerifyRuntime = true;
          }
        } catch (e1) {}
        return "rand_rt_soft_skip";
      });
    return _randVerifyModP;
  }
  /**
   * Load one Rxx template after random_rt is ready.
   * Prefer bootstrap pack_tokens opaque; API id is fixed (no withVer gen).
   */
  function ensureRandomVerifyPack(packId) {
    packId = String(packId || "");
    if (!/^R\d{2}_spotcheck$/.test(packId)) {
      return Promise.resolve("not_r");
    }
    try {
      if (global.GRCollectors && global.GRCollectors.get && global.GRCollectors.get(packId)) {
        return Promise.resolve("r_pack_present");
      }
      if (global.__GR_R_PACK_LOADED__ && global.__GR_R_PACK_LOADED__[packId]) {
        return Promise.resolve("r_pack_flag");
      }
    } catch (e0) {}
    if (_randPackP[packId]) return _randPackP[packId];
    _randPackP[packId] = ensureRandomVerifyModules()
      .then(function () {
        // 1) Standard C opaque from bootstrap pack_tokens
        var opaque =
          manifestAssetUrl("random/" + packId, "") ||
          manifestAssetUrl(packId, "") ||
          "";
        if (opaque) {
          return loadOne(opaque).then(function () {
            try {
              global.__GR_RANDOM_PACK_SOURCE__ = global.__GR_RANDOM_PACK_SOURCE__ || {};
              global.__GR_RANDOM_PACK_SOURCE__[packId] = "pack_token_opaque";
            } catch (eS) {}
            return "r_pack_opaque";
          });
        }
        // 2) Backend template API — pack id fixed; never inject ASSET_GEN into path
        var api = resolveApiBaseForR100();
        var apiUrl = api + "/v1/r100/pack/" + packId + ".js";
        return loadOne(apiUrl).then(function () {
          try {
            global.__GR_RANDOM_PACK_SOURCE__ = global.__GR_RANDOM_PACK_SOURCE__ || {};
            global.__GR_RANDOM_PACK_SOURCE__[packId] = "pseudo_static_api";
          } catch (eS2) {}
          return "r_pack_api";
        });
      })
      .then(function (how) {
        try {
          if (global.GRCollectors && global.GRCollectors.get && global.GRCollectors.get(packId)) {
            return how || "r_pack_loaded";
          }
        } catch (e1) {}
        return how || "r_pack_loaded_unchecked";
      })
      .catch(function (e) {
        try {
          try { if (global.GROps) GROps.hardLoadFail(packId, e && e.message); } catch (eOp) {}
        } catch (e2) {}
        delete _randPackP[packId];
        // Soft fail — R verify is progressive; must not kill primary residual path
        return "r_pack_fail";
      });
    return _randPackP[packId];
  }
  /** Load all R packs listed in a route_plan (usually 0–1). */
  function ensureRandomVerifyPacksForPlan(routePlan) {
    var packs = (routePlan && routePlan.packs) || [];
    var ids = [];
    for (var i = 0; i < packs.length; i++) {
      var id = String((packs[i] && (packs[i].pack_id || packs[i].id)) || "");
      if (/^R\d{2}_spotcheck$/.test(id) || (id.indexOf("spotcheck") >= 0 && /^R\d{2}/.test(id))) {
        if (ids.indexOf(id) < 0) ids.push(id);
      }
    }
    if (!ids.length) {
      return ensureRandomVerifyModules().then(function () {
        return "no_r_packs";
      });
    }
    return ensureRandomVerifyModules().then(function () {
      return Promise.all(
        ids.map(function (id) {
          return ensureRandomVerifyPack(id);
        })
      ).then(function (st) {
        try {
          global.__GR_RANDOM_PACKS_LOADED__ = ids.slice();
        } catch (e3) {}
        return st;
      });
    });
  }
  // Exposed for pack_loader.applyRoutePlan when route includes R packs.
  global.ensureRandomVerifyModules = ensureRandomVerifyModules;
  global.ensureRandomVerifyPack = ensureRandomVerifyPack;
  global.ensureRandomVerifyPacksForPlan = ensureRandomVerifyPacksForPlan;
  function ensureModules() {
    return ensureFullModules();
  }
  // Start race-module download ASAP at parse (overlap HTML/main RTT).
  try {
    ensureRaceModules();
  } catch (ePre) {}

  function resolveInjectPath(opts) {
    var c = cfg();
    var candidates = [
      global.__GR_INJECT_PRIMARY__,
      opts && opts.inject_path,
      opts && opts.injectPath,
      BOOT_ATTR_INJECT,
      c.injectPath,
      c.inject_path,
      global.__GR_INJECT_PATH__,
    ];
    // PRIMARY (nginx/cf_worker) always wins over app.
    var i;
    for (i = 0; i < candidates.length; i++) {
      if (candidates[i] === "nginx" || candidates[i] === "cf_worker") {
        global.__GR_INJECT_PRIMARY__ = candidates[i];
        return candidates[i];
      }
    }
    for (i = 0; i < candidates.length; i++) {
      if (candidates[i]) return candidates[i];
    }
    return "app";
  }

  /**
   * Resolve API base for first-party relay.
   * - Absolute https://pv… → CDN/origin (legacy)
   * - Relative /g5 or g5 → location.origin + /g5  (same-site reverse proxy)
   * - Empty + first_party flag → /g5
   */
  function resolveBaseUrl(raw) {
    var s = String(raw || "").trim();
    if (!s) return "";
    // already absolute
    if (/^https?:\/\//i.test(s)) return s.replace(/\/$/, "");
    // protocol-relative
    if (s.indexOf("//") === 0) {
      try {
        return (
          (typeof location !== "undefined" ? location.protocol : "https:") + s
        ).replace(/\/$/, "");
      } catch (e0) {
        return "https:" + s.replace(/\/$/, "");
      }
    }
    // relative first-party path
    if (s.charAt(0) !== "/") s = "/" + s;
    s = s.replace(/\/$/, "");
    try {
      if (typeof location !== "undefined" && location.origin) {
        return location.origin + s;
      }
    } catch (e1) {}
    return s;
  }

  function isFirstPartyBase(base) {
    try {
      if (!base) return false;
      if (base.charAt(0) === "/") return true;
      if (typeof location === "undefined" || !location.origin) return false;
      return new URL(base, location.href).origin === location.origin;
    } catch (e) {
      return false;
    }
  }

  function endpoint(opts) {
    var c = cfg();
    var raw =
      (opts && opts.endpoint) ||
      global.GR_ENDPOINT ||
      c.apiBase ||
      BOOT_ATTR_ENDPOINT ||
      "";
    // Prefer explicit first-party when boot cfg asks for it
    if (!raw && (c.first_party || c.firstParty || global.__GR_FIRST_PARTY__)) {
      raw = c.first_party_path || c.firstPartyPath || "/g5";
    }
    if (!raw) raw = "http://127.0.0.1:28765";
    return resolveBaseUrl(raw).replace(/\/$/, "");
  }

  /**
   * Gateway = same as upload host in prod (https://gv Pingora direct).
   * Do NOT invent /g5-gw — www plain backup is not a valid upload/B8 path.
   */
  function deriveGwBase(apiBase) {
    var base = (apiBase || "").replace(/\/$/, "");
    if (!base) return "";
    // Relative /g5 is load-only; B8 must use absolute https://gv from boot.gwBase
    if (isFirstPartyBase(base)) {
      try {
        var c0 = cfg() || {};
        var g0 = c0.gwBase || c0.gw_base || global.__GR_GW_DIRECT__ || "";
        if (g0 && /^https?:\/\//i.test(String(g0))) {
          return String(g0).replace(/\/$/, "");
        }
      } catch (e0) {}
      return "";
    }
    try {
      var u = new URL(base);
      if (u.hostname.indexOf("pv.") === 0) {
        u.hostname = "gv." + u.hostname.slice(3);
        return u.origin;
      }
      return u.origin;
    } catch (e) {
      return base;
    }
  }

  function gwEndpoint(opts, apiBase) {
    var c = cfg();
    var explicit =
      (opts && (opts.gwBase || opts.gw_base)) ||
      global.GR_GW_BASE ||
      global.__GR_GW_BASE__ ||
      global.__GR_GW_DIRECT__ ||
      c.gwBase ||
      c.gw_base ||
      BOOT_ATTR_GW ||
      "";
    if (explicit) return resolveBaseUrl(String(explicit)).replace(/\/$/, "");
    var api = apiBase || endpoint(opts);
    var derived = deriveGwBase(api);
    if (derived) return derived;
    // Unified gv: apiBase already https://gv
    if (api && /^https?:\/\//i.test(String(api))) {
      return String(api).replace(/\/$/, "");
    }
    return "";
  }

  function postJson(url, body, opts) {
    opts = opts || {};
    var sameOrigin = false;
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    var init = {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body || {}),
      // Same-origin: must send cookies (cf_clearance under CF orange). Cross-origin: include for CORS+cookies if ACAO allows.
      credentials: sameOrigin ? "same-origin" : "include",
      mode: sameOrigin ? "same-origin" : "cors",
      keepalive: !!opts.keepalive,
    };
    try {
      if (opts.priority) init.priority = opts.priority;
    } catch (e) {}
    // Abortable timeout so multi-base B8 can fall through quickly (CDN pv vs origin gv).
    var ctrl = null;
    var to = null;
    if (opts.timeout_ms && typeof AbortController !== "undefined") {
      try {
        ctrl = new AbortController();
        init.signal = ctrl.signal;
        to = setTimeout(function () {
          try {
            ctrl.abort();
          } catch (eA) {}
        }, opts.timeout_ms);
      } catch (eC) {}
    }
    return fetch(url, init)
      .then(function (r) {
        if (to) clearTimeout(to);
        return r
          .json()
          .catch(function () {
            return { ok: false, error: "HTTP " + r.status };
          })
          .then(function (j) {
            j = j || {};
            if (!r.ok || j.ok === false) {
              var err = new Error((j && j.error) || "HTTP " + r.status);
              err.status = r.status;
              err.body = j;
              err.code = j.code || j.expired_reason || "";
              err.terminal =
                r.status === 410 ||
                j.halt_uploads === true ||
                /session_expired|cycle_complete|cycle_purged|incomplete_ttl/i.test(
                  String(j.error || "") + " " + String(j.code || "")
                );
              throw err;
            }
            return j;
          });
      })
      .catch(function (e) {
        if (to) clearTimeout(to);
        throw e;
      });
  }

  /**
   * Multi-party status reconcile: FE reports local view, BE returns corrections.
   * Called after wave settle / multi-tick / when FE suspects drift (e.g. 410 lag).
   */
  function reconcileProbeStatus(apiBase, sessionId) {
    if (!apiBase || !sessionId) return Promise.resolve(null);
    var Q = global.GRUploadQueue;
    var view = Q && Q.clientView ? Q.clientView() : {
      session_id: sessionId,
      stop_probe: !!global.__GR_STOP_PROBE__,
      skip_identity: !!global.__GR_SKIP_IDENTITY__,
      halted: !!(global.__GR_UPLOAD_HALT__ || global.__GR_CYCLE_CLOSED__),
      phase: global.__GR_PHASE__ || null,
      local_uploads_done: !!global.__GR_IDENTITY_UPLOADS_DONE__,
    };
    view.session_id = sessionId;
    return postJson(
      apiBase.replace(/\/$/, "") +
        "/v1/session/" +
        encodeURIComponent(sessionId) +
        "/probe_status",
      view,
      { timeout_ms: 2500 }
    )
      .then(function (body) {
        try {
          if (Q && Q.applyCorrections) Q.applyCorrections(body);
          else if (body && body.fe_should_halt) {
            global.__GR_STOP_PROBE__ = true;
            global.__GR_SKIP_IDENTITY__ = true;
            if (Q && Q.halt) Q.halt("reconcile", sessionId, body.business_state || "halt");
          }
          global.__GR_LAST_RECONCILE__ = {
            at_ms: Date.now(),
            aligned: !!(body && body.aligned),
            body: body,
          };
        } catch (eA) {}
        return body;
      })
      .catch(function (err) {
        // 410 on probe_status itself → halt
        if (err && (err.terminal || err.status === 410)) {
          try {
            if (Q && Q.halt) Q.halt("probe_status_410", sessionId, err.code || "session_expired");
            global.__GR_STOP_PROBE__ = true;
          } catch (eH) {}
        }
        return null;
      });
  }

  /** Client-side cycle id (v57: mint before open so upload can race). */
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

  /** Parent domain so cycle/VT cookies reach both www and gv/pv (nojs→FE bond). */
  function parentCookieDomain() {
    try {
      var h = String(location.hostname || "");
      if (!h || h === "localhost" || /^\d+\.\d+\.\d+\.\d+$/.test(h) || h.indexOf(":") >= 0) {
        return "";
      }
      var parts = h.split(".");
      if (parts.length < 2) return "";
      var last = String(parts[parts.length - 1] || "").toLowerCase();
      if (
        {
          local: 1,
          test: 1,
          invalid: 1,
          localhost: 1,
          internal: 1,
          lan: 1,
          home: 1,
          corp: 1,
          localdomain: 1,
        }[last]
      ) {
        return "";
      }
      var m = h.match(/([a-z0-9-]+\.(?:com|net|org|edu|co)\.[a-z]{2})$/i);
      if (m) return "." + m[1];
      return "." + parts.slice(-2).join(".");
    } catch (e) {}
    return "";
  }

  function siteTok() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var s = boot.site_id || boot.siteId || global.__GR_SITE_ID__ || "";
      return String(s).replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 48);
    } catch (e) {
      return "";
    }
  }

  function cycleCookieName() {
    var s = siteTok();
    return s ? "gr_cycle_v1." + s : "gr_cycle_v1";
  }

  function expireCookieEverywhere(name) {
    try {
      var secure = location.protocol === "https:" ? "; secure" : "";
      document.cookie = name + "=; path=/; max-age=0; samesite=lax" + secure;
      var h = String(location.hostname || "");
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

  function writeStickyCookie(name, value, maxAge) {
    try {
      var base =
        name +
        "=" +
        encodeURIComponent(value) +
        "; path=/; max-age=" +
        (maxAge || 86400) +
        "; samesite=lax" +
        (location.protocol === "https:" ? "; secure" : "");
      var dom = parentCookieDomain();
      if (dom) {
        document.cookie = base + "; domain=" + dom;
      } else {
        document.cookie = base;
      }
    } catch (eW) {}
  }

  /** Prefer edge/CF/nginx-preseeded cycle so B8 early joins FE open/ingest. */
  function resolveCycleHint(opts) {
    var c = cfg();
    // Micro-kick / inject may already have minted a fresh cycle for this page.
    // Prefer that over sticky cookie from a *previous* completed visit (410 trap).
    var candidates = [
      opts && opts.session_id,
      opts && opts.cycle_id,
      global.__GR_CYCLE_HINT__,
      c.session_id,
      c.cycle_id,
    ];
    // Only reuse in-page session if pipeline already started with it.
    if (global.__GR_PIPELINE_STARTED__) {
      candidates.push(global.__GR_SESSION_ID__, global.__GR_CYCLE_ID__);
    }
    var i;
    for (i = 0; i < candidates.length; i++) {
      if (candidates[i] && String(candidates[i]).length >= 12) return String(candidates[i]);
    }
    // Sticky incomplete same-version cycle always preferred (gap-fill).
    // Reminting every resolve caused B8-only cycle storms with micro-kick.
    var wantVer =
      (global.__GR_BOOT__ &&
        (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
      global.__GR_PRODUCT_VERSION__ ||
      "";
    try {
      if (global.GRStorage && GRStorage.ensureProbeStateForVersion) {
        var st = GRStorage.ensureProbeStateForVersion(wantVer);
        if (st && st.cycleId && String(st.cycleId).indexOf("cycle_") === 0) {
          if (st.reminted && global.GROps && GROps.report) {
            try {
              GROps.report(
                "cycle_remint",
                "resolve_session",
                { reason: st.reason || "ensure", version: wantVer },
                "warn"
              );
            } catch (eOp) {}
          }
          return String(st.cycleId);
        }
      }
    } catch (eEns) {}
    try {
      var ckn = cycleCookieName().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      var m2 = document.cookie.match(new RegExp("(?:^|;\\s*)" + ckn + "=([^;]+)"));
      if (m2 && m2[1]) {
        var ck2 = decodeURIComponent(m2[1]);
        if (ck2.indexOf("cycle_") === 0 && ck2.length >= 12) return ck2;
      }
    } catch (eC2) {}
    try {
      if (global.GRStorage && GRStorage.getCycleId) {
        var sc2 = GRStorage.getCycleId();
        if (sc2 && String(sc2).indexOf("cycle_") === 0) return String(sc2);
      }
    } catch (eS2) {}
    // Page-level mint budget: at most one fresh mint per page load.
    try {
      if (global.__GR_PAGE_CYCLE_MINTED__) {
        return String(global.__GR_PAGE_CYCLE_MINTED__);
      }
    } catch (ePm) {}
    var minted = mintCycleId();
    try {
      global.__GR_PAGE_CYCLE_MINTED__ = minted;
    } catch (ePm2) {}
    writeStickyCookie(cycleCookieName(), minted, 86400);
    expireCookieEverywhere("gr_cycle_v1");
    expireCookieEverywhere("_g5_c");
    try {
      if (global.GRStorage && GRStorage.setCycleId) GRStorage.setCycleId(minted, wantVer);
    } catch (eSet) {}
    try {
      global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
      global.__GR_KICK_MS__.cycle_reminted = true;
      if (global.GROps && GROps.report) {
        GROps.report("cycle_remint", "resolve_session", { reason: "fresh_mint", version: wantVer }, "warn");
      }
    } catch (eK) {}
    return minted;
  }

  function mintVtInline() {
    try {
      if (global.GRStorage && GRStorage.visitorTerminalId) {
        return GRStorage.visitorTerminalId();
      }
    } catch (eSt) {}
    try {
      var keys = ["_g5_vt", "gr_visitor_terminal_v1"];
      var ki;
      for (ki = 0; ki < keys.length; ki++) {
        var v = localStorage.getItem(keys[ki]);
        if (v && v.indexOf("vt_") === 0 && v.length >= 12) {
          writeStickyCookie("_g5_vt", v, 86400 * 30);
          writeStickyCookie("gr_vt_v1", v, 86400 * 30);
          try {
            localStorage.setItem("_g5_vt", v);
            localStorage.setItem("gr_visitor_terminal_v1", v);
          } catch (eL0) {}
          return v;
        }
      }
      var cookieNames = ["_g5_vt", "gr_vt_v1"];
      for (ki = 0; ki < cookieNames.length; ki++) {
        try {
          var re = new RegExp("(?:^|;\\s*)" + cookieNames[ki] + "=([^;]+)");
          var m = document.cookie.match(re);
          if (m && m[1]) {
            var ck = decodeURIComponent(m[1]);
            if (ck.indexOf("vt_") === 0 && ck.length >= 12) {
              try {
                localStorage.setItem("_g5_vt", ck);
                localStorage.setItem("gr_visitor_terminal_v1", ck);
              } catch (eL) {}
              writeStickyCookie("_g5_vt", ck, 86400 * 30);
              writeStickyCookie("gr_vt_v1", ck, 86400 * 30);
              return ck;
            }
          }
        } catch (eCk) {}
      }
      var nid =
        "vt_" +
        Date.now().toString(36) +
        "_" +
        Math.random().toString(36).slice(2, 10);
      try {
        localStorage.setItem("_g5_vt", nid);
        localStorage.setItem("gr_visitor_terminal_v1", nid);
      } catch (eW) {}
      writeStickyCookie("_g5_vt", nid, 86400 * 30);
      writeStickyCookie("gr_vt_v1", nid, 86400 * 30);
      try {
        if (global.GROps && GROps.report) {
          GROps.report("vt_mint", "boot", { reason: "fresh_local", vt: nid }, "info");
        }
      } catch (eO) {}
      return nid;
    } catch (e) {
      return "vt_ephemeral_" + Date.now();
    }
  }

  /**
   * Early B8 without waiting for collectors (v57 runEarlyB8).
   * Dual-fire: pv CDN (join reliability) ∥ gv origin (richer TCP/TLS depth).
   * First landed wins for FE state; gv continues as fire-and-forget enrich when CDN won first.
   * Success = kicked/ingest ok (cool skip without batch is NOT success → keep retrying).
   */
  function fireEarlyB8(apiBase, gwBase, sessionId, siteId, injectPath, vt) {
    if (!sessionId) return Promise.resolve(null);
    // Single-flight per cycle: micro+boot used to dual-fire many times → ERR_ABORTED storms.
    try {
      global.__GR_B8_FLIGHT__ = global.__GR_B8_FLIGHT__ || Object.create(null);
      var flightKey = String(sessionId);
      var prev = global.__GR_B8_FLIGHT__[flightKey];
      if (prev && prev.then) return prev;
      // Already landed for this cycle — skip re-fire (unless forced enrich later).
      if (global.__GR_B8_LANDED__ === flightKey) {
        return Promise.resolve(global.__GR_EARLY_B8__ || { ok: true, kicked: "B8_gateway", deduped: true });
      }
    } catch (eFl) {}

    var api = (apiBase || "").replace(/\/$/, "");
    var gw = (gwBase || deriveGwBase(api) || "").replace(/\/$/, "");
    // Classify first-party vs edge: prefer same-origin /g5 first (no CORS), then edge enrich.
    var fpBase = null;
    var edgeBase = null;
    try {
      if (api) {
        if (api.charAt(0) === "/") {
          fpBase = api;
        } else if (typeof location !== "undefined" && api.indexOf(location.origin) === 0) {
          fpBase = api;
        } else {
          // Absolute cross-origin API — treat as edge-like until we have a relative path.
          edgeBase = api;
        }
      }
    } catch (eFp) {
      if (api && api.charAt(0) === "/") fpBase = api;
      else if (api) edgeBase = api;
    }
    if (gw && gw !== api && gw !== edgeBase) {
      // Real Pingora TLS gateway for TCP/TLS depth.
      edgeBase = gw;
    }
    // If gw is same host as page, fold into first-party only.
    try {
      if (edgeBase && typeof location !== "undefined" && edgeBase.indexOf(location.host) >= 0) {
        fpBase = fpBase || edgeBase;
        edgeBase = null;
      }
    } catch (eH) {}
    if (!fpBase && !edgeBase) return Promise.resolve(null);

    var body = {
      session_id: sessionId,
      visitor_terminal_id: vt || undefined,
      site_id: siteId || undefined,
      inject_path: injectPath || "nginx",
      fields: {
        early_kick: true,
        kicked_ms: Date.now(),
        user_agent: typeof navigator !== "undefined" ? navigator.userAgent : "",
        b8_bases: (fpBase ? 1 : 0) + (edgeBase ? 1 : 0),
        b8_dual: !!(fpBase && edgeBase),
      },
    };

    function b8Ok(j) {
      if (!j || j.ok === false) return false;
      if (j.skip_identity_probe && !j.kicked) return false;
      if (j.kicked === "B8_gateway") return true;
      if (j.ingest && (j.ingest.ok === true || j.ingest.batch_id === "B8_gateway")) return true;
      return !!(j.session_id && j.kicked);
    }

    function tryBase(base, kind, timeoutMs, noAbort) {
      var b = Object.assign({}, body, {
        fields: Object.assign({}, body.fields, {
          b8_path_kind: kind,
        }),
      });
      var opts = {
        priority: "high",
        keepalive: true,
      };
      // Avoid AbortController on enrich path — aborted dual-fire flooded console with ERR_ABORTED.
      if (!noAbort) opts.timeout_ms = timeoutMs || 3500;
      return postJson(base + "/v1/gateway/early", b, opts).then(function (j) {
        if (!b8Ok(j)) throw new Error("b8_not_landed");
        try {
          j.__b8_via = base;
          j.__b8_kind = kind;
        } catch (eV) {}
        return j;
      });
    }

    function beaconFallback() {
      // Prefer fetch/sendBeacon over <img>: site CSP often omits pv/gv from img-src
      // (zhanso: img-src only self + Google ad domains) which blocks Image() beacons.
      //
      // sendBeacon always POSTs. Empty body used to 400 on gateway ("requires session_id")
      // because POST only read JSON body. Ship JSON Blob + query so both paths land B8.
      var q =
        "session_id=" +
        encodeURIComponent(sessionId) +
        "&inject_path=" +
        encodeURIComponent(injectPath || "nginx") +
        (siteId ? "&site_id=" + encodeURIComponent(siteId) : "") +
        (vt ? "&visitor_terminal_id=" + encodeURIComponent(vt) : "") +
        "&e=b8_beacon&early_kick=1";
      var beaconBody = {
        session_id: sessionId,
        visitor_terminal_id: vt || undefined,
        site_id: siteId || undefined,
        inject_path: injectPath || "nginx",
        fields: {
          early_kick: true,
          via: "b8_beacon",
          kicked_ms: Date.now(),
          user_agent: typeof navigator !== "undefined" ? navigator.userAgent : "",
        },
      };
      var urls = [];
      if (gw) urls.push(gw + "/s0?" + q);
      if (api && api !== gw) urls.push(api + "/s0?" + q);
      urls.forEach(function (u) {
        var sent = false;
        try {
          if (typeof navigator !== "undefined" && typeof navigator.sendBeacon === "function") {
            var blob = new Blob([JSON.stringify(beaconBody)], {
              type: "application/json",
            });
            sent = !!navigator.sendBeacon(u, blob);
          }
        } catch (eSb) {
          sent = false;
        }
        // GET carries session_id in query (s0_get_compat) — reliable when sendBeacon blocked.
        if (!sent) {
          try {
            fetch(u, {
              method: "GET",
              mode: "no-cors",
              credentials: "omit",
              keepalive: true,
              cache: "no-store",
            }).catch(function () {});
            sent = true;
          } catch (eF) {
            sent = false;
          }
        }
        // Last resort Image — may still be CSP-blocked; swallow only.
        if (!sent) {
          try {
            var img = new Image();
            img.referrerPolicy = "no-referrer";
            img.src = u;
          } catch (eImg) {}
        }
      });
      return null;
    }

    /**
     * Short-visit gateway: primary first-party + edge enrich **in parallel** (no AbortController
     * on enrich → no ERR_ABORTED spam). Edge starts immediately so TCP/TLS depth lands early.
     * Primary uses short timeout (2.5s) so fallback is fast on slow /g5.
     */
    var flight = (function () {
      var primary = fpBase
        ? { base: fpBase, kind: "cdn" }
        : { base: edgeBase, kind: "edge" };
      var secondary =
        fpBase && edgeBase
          ? { base: edgeBase, kind: "edge" }
          : null;

      function markLanded(j) {
        try {
          global.__GR_B8_LANDED__ = String(sessionId);
          global.__GR_EARLY_B8__ = j;
          global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
          global.__GR_KICK_MS__.b8_via = (j && j.__b8_via) || primary.base;
          global.__GR_KICK_MS__.b8_kind = (j && j.__b8_kind) || primary.kind;
        } catch (eM) {}
      }

      function enrichEdge() {
        if (!secondary) return;
        // Fire-and-forget: no timeout abort → no console ERR_ABORTED spam.
        tryBase(secondary.base, secondary.kind, 0, true)
          .then(function () {
            try {
              global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
              global.__GR_KICK_MS__.b8_edge_enrich = true;
            } catch (eE) {}
          })
          .catch(function () {});
      }

      // Parallel edge enrich while primary lands (short-visit win).
      enrichEdge();

      return tryBase(primary.base, primary.kind, 2500, false)
        .then(function (j) {
          markLanded(j);
          return j;
        })
        .catch(function () {
          // Primary failed: try edge as primary once (with soft timeout).
          if (secondary) {
            return tryBase(secondary.base, secondary.kind, 3000, false)
              .then(function (j) {
                markLanded(j);
                return j;
              })
              .catch(function () {
                beaconFallback();
                return null;
              });
          }
          beaconFallback();
          return null;
        });
    })();
    try {
      global.__GR_B8_FLIGHT__ = global.__GR_B8_FLIGHT__ || Object.create(null);
      global.__GR_B8_FLIGHT__[String(sessionId)] = flight;
      flight.then(function () {
        try {
          delete global.__GR_B8_FLIGHT__[String(sessionId)];
        } catch (eD) {}
      });
    } catch (eF2) {}
    return flight;
  }

  /** True when early B8 actually landed a gateway batch (not cool-skip). */
  function earlyB8Landed(b8) {
    if (!b8 || b8.ok === false) return false;
    if (b8.skip_identity_probe && !b8.kicked) return false;
    return b8.kicked === "B8_gateway" || !!(b8.ingest && b8.ingest.ok);
  }

  function rpaEnabledForPlan() {
    try {
      var policy = global.__GR_RESULT_POLICY__;
      if (policy && policy.rpa_collect === false) return false;
    } catch (eR) {}
    // RPA collection is on by default. Only the explicit result policy
    // disables it; compatibility entitlement metadata is not a capability gate.
    return true;
  }

  function applyOpenEntitlement(opened) {
    try {
      var ent = (opened && opened.entitlement) || {};
      var policy =
        (opened && opened.result_policy) ||
        (opened && opened.policy && opened.policy.result_policy) ||
        {};
      var enabled = policy.rpa_collect !== false;
      global.__GR_ENTITLEMENT__ = ent;
      global.__GR_RESULT_POLICY__ = policy;
      global.__GR_RPA_ENABLED__ = !!enabled;
      global.__GR_DEVICE_PRECISIONS__ =
        (opened && opened.device_precisions) || ent.device_precisions || [];
    } catch (eA) {}
  }

  /**
   * B11 early: bind main RPA ASAP after B0 (do not wait wave2/registry).
   * Uses rpa_monitor when present; otherwise lightweight inline listeners + queue flush.
   * Result policy may disable RPA collection without disabling other probes.
   */
  function bindEarlyB11(ctx) {
    if (!rpaEnabledForPlan()) {
      try {
        global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
        global.__GR_KICK_MS__.b11_early = "plan_disabled";
      } catch (eD) {}
      return;
    }
    if (!ctx || global.__GR_RPA_BOUND__ || pageUnloading()) return;
    var page_id = (ctx && (ctx.page_id || (ctx.fields && ctx.fields.page_id))) || null;
    var page_url = "";
    try {
      page_url = String(location.href || "");
    } catch (eU) {}
    if (global.GRRpaMonitor && typeof global.GRRpaMonitor.bind === "function") {
      try {
        var st = global.GRRpaMonitor.bind({
          source: "main",
          session_id: ctx.session_id,
          page_id: page_id,
          page_url: page_url,
          inject_path: ctx.inject_path,
          queue: ctx.queue,
          win: global,
          doc: typeof document !== "undefined" ? document : null,
          boundKey: "__GR_RPA_BOUND_main__",
        });
        global.__GR_RPA_BOUND__ = true;
        global.__GR_RPA_EVENTS__ = (st && st.events) || [];
        global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
        global.__GR_KICK_MS__.b11_early = "rpa_monitor";
        try {
          if (global.GRPackLoader && GRPackLoader.markKicked) {
            GRPackLoader.markKicked("B11_interaction");
          }
        } catch (eK) {}
        return;
      } catch (eR) {}
    }
    // Inline bind (same contract as registry fallback)
    var events = global.__GR_RPA_EVENTS__ || [];
    global.__GR_RPA_EVENTS__ = events;
    global.__GR_RPA_BOUND__ = true;
    function flushRpa(reason, force) {
      var now = Date.now();
      var unloading = reason === "pagehide" || reason === "beforeunload" || reason === "close";
      var background = reason === "hidden" || reason === "visibility_hidden";
      if (unloading) {
        markPageUnloading();
      } else if (background) {
        markPageBackgrounded(true);
      }
      var hiding = unloading || background;
      var fields = {
        behavior_early_bound: true,
        behavior_events: events.slice(),
        behavior_count: events.length,
        pagehide_flush: !!hiding,
        behavior_pagehide: !!hiding,
        rpa_flush_reason: reason || "tick",
        rpa_idle_flush: reason === "idle_45s" || reason === "idle_30s",
        page_id: page_id,
        page_url: page_url,
        collected_at: now,
        rpa_source: "main",
        sandbox_kind: "main",
        b11_early: true,
      };
      global.__GR_RPA_LAST_UPLOAD_MS__ = now;
      try {
        if (ctx.queue && ctx.queue.enqueue) {
          ctx.queue.enqueue({
            session_id: ctx.session_id,
            batch_id: "B11_interaction",
            source: "main",
            inject_path: ctx.inject_path,
            priority: hiding ? 100 : 88,
            force: !!force || hiding,
            payload: { fields: fields, sandbox_kind: "main" },
          });
        }
      } catch (eQ) {}
    }
    function push(kind, ev) {
      if (events.length >= 80) events.shift();
      events.push({
        t: Date.now(),
        kind: kind,
        type: kind,
        source: "main",
        x: (ev && ev.clientX) || 0,
        y: (ev && ev.clientY) || 0,
      });
      global.__GR_RPA_LAST_EVENT_MS__ = Date.now();
    }
    try {
      ["pointerdown", "pointermove", "pointerup", "scroll", "keydown", "keyup", "click", "touchstart"].forEach(
        function (evName) {
          document.addEventListener(
            evName,
            function (ev) {
              push(evName, ev);
            },
            { passive: true, capture: true }
          );
        }
      );
      // Coalesce: continuous RPA every 8s (was 2s) — still maximize signal, less ingest spam.
      global.__GR_RPA_TICK__ = setInterval(function () {
        if (pageUnloading()) return;
        if (global.__GR_STOP_PROBE__ || global.__GR_HALT_UPLOADS__) return;
        var t = Date.now();
        var lastE = global.__GR_RPA_LAST_EVENT_MS__ || 0;
        var lastU = global.__GR_RPA_LAST_UPLOAD_MS__ || 0;
        var contMs = 8000;
        var nCont = global.__GR_B11_CONT_N__ || 0;
        if (nCont >= 8) {
          // Cap continuous flushes per cycle; idle path still allowed once.
          if (events.length && lastU && t - lastU >= 45000 && t - lastE >= 45000) {
            flushRpa("idle_45s", false);
          }
          return;
        }
        if (events.length && lastE > lastU && t - lastU >= contMs) {
          global.__GR_B11_CONT_N__ = nCont + 1;
          flushRpa("continuous", false);
        } else if (events.length && lastU && t - lastU >= 45000 && t - lastE >= 45000) {
          flushRpa("idle_45s", false);
        }
      }, 2000);
      var onUnload = function () {
        flushRpa("pagehide", true);
      };
      global.addEventListener("pagehide", onUnload);
      global.addEventListener("beforeunload", onUnload);
      global.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "hidden") {
          flushRpa("hidden", true);
        } else if (document.visibilityState === "visible") {
          markPageVisible();
        }
      });
      // First tick: bind marker batch so server knows RPA is live
      flushRpa("bind", false);
      global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
      global.__GR_KICK_MS__.b11_early = "inline";
      try {
        if (global.GRPackLoader && GRPackLoader.markKicked) {
          GRPackLoader.markKicked("B11_interaction");
        }
      } catch (eK2) {}
    } catch (eBind) {
      global.__GR_RPA_BOUND__ = false;
    }
  }

  function reportBizVisit(apiBase, payload) {
    if (!payload || !payload.site_id || !payload.visitor_terminal_id) {
      return Promise.resolve(null);
    }
    // FE must not embed raw SDK secrets. Skip client biz/visit when no key is
    // configured — open/pixel already upsert biz server-side when available.
    // Avoids console 401 noise on local-fp / lab without admin SDK keys.
    var sdkKey = null;
    try {
      sdkKey =
        (global.__GR_BOOT__ && global.__GR_BOOT__.sdkKey) ||
        global.__GR_SDK_KEY__ ||
        null;
    } catch (eK) {}
    if (!sdkKey) {
      return Promise.resolve({ skipped: true, reason: "no_fe_sdk_key" });
    }
    var headers = { "content-type": "application/json" };
    headers["X-Gr-Sdk-Key"] = String(sdkKey);
    // biz/visit: same-origin keeps CF cookies; cross-origin include when CORS allows credentials
    var bizUrl = String(apiBase || "").replace(/\/$/, "") + "/v1/biz/visit";
    var bizCm = fetchCredMode(bizUrl);
    return fetch(bizUrl, {
      method: "POST",
      headers: headers,
      body: JSON.stringify(payload),
      credentials: bizCm.credentials,
      mode: bizCm.mode,
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .catch(function () {
        return null;
      });
  }

  /**
   * CF orange / Under Attack sets cf_clearance on the document host.
   * same-origin XHR must use credentials:"same-origin" or challenge cookies are omitted → 403 HTML.
   * Cross-origin (pv/gv) cannot send www's cf_clearance; those hosts need separate CF policy or grey cloud.
   */
  function fetchCredMode(url) {
    try {
      var same =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
      return same
        ? { credentials: "same-origin", mode: "same-origin" }
        : { credentials: "include", mode: "cors" };
    } catch (e) {
      var rel = String(url || "").charAt(0) === "/";
      return rel
        ? { credentials: "same-origin", mode: "same-origin" }
        : { credentials: "include", mode: "cors" };
    }
  }

  function getJson(url) {
    var cm = fetchCredMode(url);
    return fetch(url, {
      method: "GET",
      credentials: cm.credentials,
      mode: cm.mode,
      cache: "no-store",
    }).then(function (r) {
      return r.json().then(function (j) {
        if (!r.ok || j.ok === false) throw new Error((j && j.error) || "HTTP " + r.status);
        return j;
      });
    });
  }

  /** POST JSON with CF-safe credentials (preferred under orange cloud for poll paths). */
  function postJsonPoll(url, body) {
    var cm = fetchCredMode(url);
    return fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body || {}),
      credentials: cm.credentials,
      mode: cm.mode,
      cache: "no-store",
    }).then(function (r) {
      return r.json().then(function (j) {
        if (!r.ok || j.ok === false) throw new Error((j && j.error) || "HTTP " + r.status);
        return j;
      });
    });
  }

  /** Latest analysis result from server (auto-analyze owned). Prefer POST then GET. */
  function fetchLatestAnalysis(apiBase, sessionId) {
    var base = String(apiBase || "").replace(/\/$/, "");
    var path = base + "/v1/session/" + encodeURIComponent(sessionId) + "/analyses";
    var boot = cfg() || {};
    var poll = String(boot.poll_method || boot.pollMethod || "both").toLowerCase();
    function pick(j) {
      var arr = (j && j.analyses) || [];
      if (!arr.length) return null;
      var last = arr[arr.length - 1];
      return last.result || last;
    }
    if (poll === "get") return getJson(path).then(pick);
    if (poll === "post") return postJsonPoll(path, { session_id: sessionId }).then(pick);
    // both: POST first (CF/WAF often softer on POST after clearance), fall back GET
    return postJsonPoll(path, { session_id: sessionId })
      .then(pick)
      .catch(function () {
        return getJson(path).then(pick);
      });
  }

  function waitUploadIdle(timeoutMs) {
    var Q = global.GRUploadQueue;
    if (Q && Q.whenIdle) return Q.whenIdle(timeoutMs == null ? 1500 : timeoutMs);
    return new Promise(function (resolve) {
      setTimeout(function () {
        resolve(Q ? Q.stats() : {});
      }, Math.min(timeoutMs || 80, 80));
    });
  }

  function bindHide(Q) {
    if (global.__GR_FLUSH_BOUND__) return;
    global.__GR_FLUSH_BOUND__ = true;
    function unloadFlush() {
      markPageUnloading();
      Q.flush("pagehide");
      // Best-effort: trigger analyze with pagehide flag (does not overwrite probe batches).
      try {
        var api = (Q && Q.cfg && Q.cfg.apiBase) || (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) || "";
        var sid = (Q && Q.cfg && Q.cfg.session_id) || global.__GR_SESSION_ID__ || "";
        if (api && sid && typeof fetch === "function") {
          var aUrl =
            String(api).replace(/\/$/, "") +
            "/v1/session/" +
            encodeURIComponent(sid) +
            "/analyze";
          var aCm = fetchCredMode(aUrl);
          fetch(aUrl, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ pagehide_flush: true }),
            keepalive: true,
            credentials: aCm.credentials,
            mode: aCm.mode,
          }).catch(function () {});
        }
      } catch (e2) {}
    }
    function backgroundFlush() {
      // Tab not focused — prioritize hard uploads but KEEP probing (maximize).
      markPageBackgrounded(true);
      Q.flush("hidden");
    }
    function onVisible() {
      markPageVisible();
      // Resume hard SLA if B10 still missing after focus return.
      try {
        if (global.__GR_REQUEST_HARD_SLA__) global.__GR_REQUEST_HARD_SLA__("visible");
      } catch (eV) {}
      // Self-heal supervisor: visible → immediate T/C/B pass.
      try {
        if (global.GRProbeSelfHeal && GRProbeSelfHeal.onVisible) {
          GRProbeSelfHeal.onVisible();
        }
      } catch (eSh) {}
    }
    global.addEventListener("pagehide", unloadFlush);
    global.addEventListener("beforeunload", unloadFlush);
    document.addEventListener("visibilitychange", function () {
      if (document.visibilityState === "hidden") backgroundFlush();
      else if (document.visibilityState === "visible") onVisible();
    });
  }

  /**
   * Multi-tick brain loop while session open.
   * Server debounce auto-analyze is the source of truth; FE polls /analyses and applies route_plan.
   * Falls back to one explicit /analyze only if queue is idle and no rev yet.
   * @param {object} ctx
   * @param {number} maxTicks
   * @param {number} idleTimeoutMs max wait for upload queue idle per tick
   */
  function multiTickLoop(ctx, maxTicks, idleTimeoutMs) {
    // iss/70: multi_tick_max is abnormal safety only (default 16).
    // Normal stop: server terminal, plan_epoch unchanged + empty kicks, B10+ACK rich.
    var pol =
      (global.GRProbeLifecycle && global.GRProbeLifecycle.POLICY) || {};
    maxTicks =
      maxTicks == null
        ? pol.multi_tick_max != null
          ? pol.multi_tick_max
          : 16
        : maxTicks;
    // Hard ceiling even if misconfigured high.
    if (maxTicks > 24) maxTicks = 24;
    // Single active multi-tick per session (prevent stacked loops).
    try {
      var loopKey = "mt:" + String((ctx && ctx.session_id) || "");
      if (global.__GR_MULTI_TICK_ACTIVE__ === loopKey) {
        return Promise.resolve([{ n: 0, skipped: true, reason: "multi_tick_already_active" }]);
      }
      global.__GR_MULTI_TICK_ACTIVE__ = loopKey;
    } catch (eMt) {}
    // Short-visit: don't wait long for queue idle before next brain tick.
    idleTimeoutMs = idleTimeoutMs == null ? 700 : idleTimeoutMs;
    var emptyPatienceBase =
      pol.empty_kick_patience != null ? pol.empty_kick_patience : 5;
    var emptyPatiencePostB10 =
      pol.empty_kick_patience_post_b10 != null ? pol.empty_kick_patience_post_b10 : 3;
    var emptyPatience = emptyPatienceBase;
    var ticks = [];
    var i = 0;
    var lastRev = 0;
    var emptyKicks = 0;
    var lastPlanEpoch = null;
    var samePlanEpochStreak = 0;

    function sleep(ms) {
      return new Promise(function (resolve) {
        setTimeout(resolve, ms);
      });
    }

    /**
     * iss/72: distinguish collected/uploading vs ACKED.
     * Empty-stop patience only tightens on verified B10 ACK (or server cov.has_b10).
     */
    function b10TransportState() {
      try {
        if (
          global.GRUploadQueue &&
          typeof GRUploadQueue.materialState === "function"
        ) {
          return GRUploadQueue.materialState(
            "B10_hw_curves",
            ctx.session_id,
            "main"
          );
        }
      } catch (e) {}
      return "absent";
    }
    function hasB10Acked() {
      try {
        var mst = b10TransportState();
        if (mst === "acked") return true;
        if (global.GRUploadQueue && GRUploadQueue.alreadySent) {
          if (
            GRUploadQueue.alreadySent({
              session_id: ctx.session_id,
              batch_id: "B10_hw_curves",
              source: "main",
            })
          ) {
            return true;
          }
        }
        var la = global.__GR_LAST_ANALYZE__;
        var res = la && (la.result || la);
        var cov = (res && res.coverage) || {};
        var cps = (res && res.cycle_probe_status) || {};
        if (cov.has_b10 === true || cps.has_b10 === true) return true;
        var rec = global.__GR_RECEIVED_BATCHES__ || [];
        for (var hi = 0; hi < rec.length; hi++) {
          if (String(rec[hi].batch_id || "") === "B10_hw_curves") return true;
        }
      } catch (eB10) {}
      return false;
    }
    function hasB10InFlight() {
      var mst = b10TransportState();
      return mst === "uploading" || mst === "queued" || mst === "retry_wait";
    }
    /** Back-compat name: true if B10 collected OR acked OR in-flight (for poll spacing). */
    function clientHasB10() {
      return hasB10Acked() || hasB10InFlight();
    }

    function refreshEmptyPatience() {
      // iss/72: only ACKED (or server has_b10) tightens empty patience.
      emptyPatience = hasB10Acked() ? emptyPatiencePostB10 : emptyPatienceBase;
    }

    /**
     * Poll until a new analysis rev appears or attempts exhausted.
     * After B10 lands, use fewer polls + larger gap (was 12×40ms → ~185 analyses/55s).
     * Maximize still runs many ticks — we just stop hammering /analyses.
     */
    function pollNewAnalysis(attempts, gapMs) {
      var hasB10 = clientHasB10();
      if (attempts == null) attempts = hasB10 ? 2 : 6;
      if (gapMs == null) gapMs = hasB10 ? 700 : 120;
      var n = 0;
      function step() {
        return fetchLatestAnalysis(ctx.apiBase, ctx.session_id).then(function (result) {
          if (result) {
            var rev = result.analysis_rev || result.rev || 0;
            if (rev > lastRev || (!lastRev && result.real_band)) {
              return result;
            }
          }
          n++;
          if (n >= attempts) return result || null;
          return sleep(gapMs).then(step);
        });
      }
      return step();
    }

    function applyServerTerminal(result, source) {
      if (!result) return false;
      var cps = result.cycle_probe_status || {};
      var cov = cps.identity_coverage || result.identity_coverage || {};
      // Prefer hardFinalComplete (brain schedule final + silicon). dh_ alone is NOT final.
      var hardOk = false;
      try {
        if (global.GRUploadQueue && typeof global.GRUploadQueue.hardFinalComplete === "function") {
          hardOk = !!global.GRUploadQueue.hardFinalComplete(result, cps);
        } else if (
          cov.brain_schedule_final === true ||
          cps.brain_schedule_final === true ||
          result.brain_schedule_final === true ||
          cov.final_analysis_ok === true ||
          cps.final_analysis_ok === true ||
          result.final_analysis_ok === true
        ) {
          hardOk = true;
        } else {
          // Fallback: brain terminal + coverage, never commercial milestone alone.
          var coverageComplete =
            (result.coverage && result.coverage.coverage_complete === true) ||
            cps.coverage_complete === true ||
            cov.coverage_complete === true;
          var stopProbe =
            result.stop_probe === true ||
            (result.route_plan && result.route_plan.stop_probe === true);
          hardOk =
            (result.analysis_terminal === true || cps.analysis_terminal === true || (stopProbe && coverageComplete)) &&
            (cov.has_b10 === true || cps.has_b10 === true);
        }
      } catch (eHf) {}
      var closed =
        cps.cycle_closed === true ||
        cps.cycle_status === "complete" ||
        cps.cycle_status === "purged" ||
        !!result.cycle_complete ||
        result.cycle_closes === true;
      // Thin halt_uploads without schedule final must NOT halt (continue soft/mid).
      var halt =
        closed ||
        hardOk ||
        ((result.halt_uploads === true || cps.halt_uploads === true) && (hardOk || closed));
      if (!halt) return false;
      global.__GR_STOP_PROBE__ = true;
      global.__GR_SKIP_IDENTITY__ = true;
      global.__GR_HALT_UPLOADS__ = true;
      global.__GR_HARD_FINAL__ = true;
      global.__GR_BRAIN_SCHEDULE_FINAL__ = true;
      global.__GR_PHASE__ = "cool";
      try {
        if (global.GRProbeLifecycle && GRProbeLifecycle.markCool) {
          GRProbeLifecycle.markCool(source || "terminal");
        }
      } catch (eLc) {}
      try {
        global.__GR_CYCLE_PROBE_STATUS__ = cps.algo ? cps : result.cycle_probe_status || cps;
        global.__GR_BUSINESS_STATE__ =
          cps.business_state || result.business_state || "identity_complete_cool";
        // Persist cool immediately so refresh does not re-kick packs / flood 410.
        // Prefer server cool_until_ms; else 24h local cool (matches CYCLE_COOL_MS).
        try {
          var coolMs =
            result.cool_until_ms ||
            (result.session && result.session.cool_until_ms) ||
            cps.cool_until_ms ||
            null;
          if (global.GRStorage && GRStorage.setCoolUntil) {
            GRStorage.setCoolUntil(
              coolMs || Date.now() + 24 * 60 * 60 * 1000,
              (result && result.product_version) ||
                (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
                global.__GR_PRODUCT_VERSION__
            );
          }
        } catch (eCoolT) {}
        var Q = global.GRUploadQueue;
        if (Q && Q.halt) {
          Q.halt(
            source || "analyze_terminal",
            ctx.session_id,
            cps.expired_reason || result.code || "analysis_terminal"
          );
        }
      } catch (eH) {}
      return true;
    }

    function oneTick() {
      if (
        i >= maxTicks ||
        global.__GR_STOP_PROBE__ ||
        pageUnloading() ||
        global.__GR_CYCLE_CLOSED__
      ) {
        return Promise.resolve(ticks);
      }
      i++;
      return waitUploadIdle(idleTimeoutMs)
        .then(function () {
          // Tick1: quicker wait for first analyze; later: sparse poll (esp. post-B10).
          return pollNewAnalysis(
            i === 1 ? 8 : clientHasB10() ? 2 : 4,
            i === 1 ? 100 : clientHasB10() ? 800 : 250
          );
        })
        .then(function (result) {
          // First tick only: if server auto-analyze still empty, one explicit analyze.
          if (!result && i === 1) {
            return postJson(
              ctx.apiBase + "/v1/session/" + encodeURIComponent(ctx.session_id) + "/analyze",
              {} // A-BRAIN-3: server owns soft_v2_ready; client must not claim Soft
            ).then(function (aj) {
              if (aj && (aj.halt_uploads || aj.analysis_terminal || aj.cycle_closes)) {
                applyServerTerminal(aj, "analyze_response");
                return {
                  __terminal: true,
                  analysis_terminal: true,
                  halt_uploads: true,
                  cycle_probe_status: aj.cycle_probe_status,
                };
              }
              return aj.result || aj;
            }).catch(function (err) {
              // postJson throws on 410 with err.terminal — cycle closed, not "empty poll".
              if (err && (err.terminal || err.status === 410)) {
                try {
                  applyServerTerminal(err.body || { halt_uploads: true, code: err.code }, "analyze_410");
                } catch (eA) {
                  global.__GR_STOP_PROBE__ = true;
                  try {
                    var Q2 = global.GRUploadQueue;
                    if (Q2 && Q2.halt) Q2.halt("analyze_410", ctx.session_id, err.code || "session_expired");
                  } catch (e2) {}
                }
                return { __terminal: true, analysis_terminal: true, halt_uploads: true };
              }
              return null;
            });
          }
          return result;
        })
        .then(function (result) {
          if (result && result.__terminal) {
            ticks.push({ n: i, terminal: true, mode: "halt" });
            return ticks;
          }
          if (applyServerTerminal(result, "multi_tick")) {
            ticks.push({
              n: i,
              mode: "poll",
              analysis_terminal: true,
              stop_probe: true,
              coverage_complete: true,
              halted: true,
            });
            return ticks;
          }
          if (result && !ctx.__biz_analyze_reported) {
            ctx.__biz_analyze_reported = true;
            reportBizVisit(ctx.apiBase, {
              site_id: resolveSiteId({}) || global.__GR_SITE_ID__ || "",
              visitor_terminal_id:
                global.__GR_VTID__ ||
                (global.GRStorage && GRStorage.visitorTerminalId()) ||
                "",
              visitor_facet: global.__GR_VISITOR_FACET__ || "browser",
              session_id: ctx.session_id,
              page_host:
                typeof location !== "undefined" ? location.hostname || "" : "",
              event: "analyze_ready",
              events: ["analyze_ready"],
              summary: {
                source: "gr.boot",
                real_band: result.real_band || null,
                bot_verdict: result.bot_verdict || null,
              },
            });
          }
          return result;
        })
        .then(function (result) {
          if (!result) {
            ticks.push({ n: i, empty: true, mode: "poll" });
            return ticks;
          }
          var rev = result.analysis_rev || result.rev || lastRev + 1;
          lastRev = Math.max(lastRev, rev);
          var plan =
            result.route_plan ||
            (result.brain && result.brain.route_plan) ||
            null;
          var cov = result.coverage || (result.brain && result.brain.coverage) || {};
          global.__GR_ROUTE_PLAN__ = plan;
          // iss/61 F1: persist fuzzy Helper Data for next-session echo (parity-only).
          try {
            if (plan && plan.fuzzy_helper && global.GRStorage && GRStorage.setFuzzyHelper) {
              GRStorage.setFuzzyHelper(plan.fuzzy_helper);
            }
          } catch (eFh) {}
          global.__GR_LAST_ANALYZE__ = { result: result, route_plan: plan };
          refreshEmptyPatience();
          var planEpoch =
            (plan && (plan.plan_epoch != null ? plan.plan_epoch : plan.plan_version)) ||
            result.plan_epoch ||
            result.plan_version ||
            rev;
          // iss/72: SessionScheduler accepts only fresh plan_epoch; drop stale plans.
          try {
            if (global.GRSessionScheduler) {
              if (ctx && ctx.session_id && GRSessionScheduler.setSession) {
                GRSessionScheduler.setSession(ctx.session_id);
              }
              if (plan && typeof GRSessionScheduler.acceptRoutePlan === "function") {
                var acc = GRSessionScheduler.acceptRoutePlan(plan, rev);
                if (acc && acc.ok === false && acc.reason === "stale_plan_epoch") {
                  ticks.push({
                    n: i,
                    mode: "poll",
                    analysis_rev: rev,
                    plan_epoch: planEpoch,
                    stale_plan: true,
                    packs_planned: 0,
                    dynamic_kicked: [],
                  });
                  return sleep(clientHasB10() ? 600 : 350).then(oneTick);
                }
              } else if (GRSessionScheduler.setPlanEpoch) {
                GRSessionScheduler.setPlanEpoch(planEpoch);
              }
            }
            global.__GR_PLAN_EPOCH__ = planEpoch;
          } catch (eSch) {}
          // iss/70: same plan_epoch with no new packs → count as empty (stop thrash).
          if (lastPlanEpoch != null && String(planEpoch) === String(lastPlanEpoch)) {
            samePlanEpochStreak++;
          } else {
            samePlanEpochStreak = 0;
            lastPlanEpoch = planEpoch;
          }
          var tick = {
            n: i,
            mode: "poll",
            analysis_rev: rev,
            plan_version: (plan && plan.plan_version) || result.plan_version || i,
            plan_epoch: planEpoch,
            real_band: result.real_band,
            stop_probe: !!(plan && plan.stop_probe),
            coverage_complete: !!(cov.coverage_complete || result.analysis_terminal),
            packs_planned: plan && plan.packs ? plan.packs.length : 0,
            dynamic_kicked: [],
          };
          // Only stop on same-epoch when B10 is ACKED (not merely queued).
          if (samePlanEpochStreak >= emptyPatience && hasB10Acked()) {
            tick.same_plan_stop = true;
            ticks.push(tick);
            try {
              global.__GR_MULTI_TICK_ACTIVE__ = null;
            } catch (eClr0) {}
            return ticks;
          }
          // Server schedule final only (not dh_ milestone). See FE_BE_STATE_MACHINE_AUTHORITY_V5855.
          if (applyServerTerminal(result, "multi_tick_poll")) {
            tick.stop_probe = true;
            tick.coverage_complete = true;
            tick.schedule_final = true;
            ticks.push(tick);
            return ticks;
          }
          // Terminal when brain says stop_probe AND coverage_complete.
          if (plan && plan.stop_probe && tick.coverage_complete) {
            global.__GR_STOP_PROBE__ = true;
            ticks.push(tick);
            return ticks;
          }
          if (!plan) {
            ticks.push(tick);
            emptyKicks++;
            // No plan yet: wait for analyze — do not treat as complete.
            if (emptyKicks >= emptyPatience) return ticks;
            return sleep(clientHasB10() ? 600 : 350).then(oneTick);
          }
          // PackLoader may still be loading mid/dense/R after L1 — wait patiently.
          if (!global.GRPackLoader || typeof GRPackLoader.applyRoutePlan !== "function") {
            ticks.push(tick);
            emptyKicks++;
            if (emptyKicks >= emptyPatience) return ticks;
            return sleep(clientHasB10() ? 500 : 280).then(oneTick);
          }
          // Skip re-apply when plan epoch unchanged and last kick was empty.
          if (samePlanEpochStreak > 0 && emptyKicks > 0) {
            ticks.push(tick);
            emptyKicks++;
            if (emptyKicks >= emptyPatience) {
              try {
                global.__GR_MULTI_TICK_ACTIVE__ = null;
              } catch (eClr1) {}
              return ticks;
            }
            return sleep(clientHasB10() ? 700 : 400).then(oneTick);
          }
          // Keep Gap Table in sync with every brain plan (even if multi_tick later exits).
          try {
            if (global.GRProbeSelfHeal && GRProbeSelfHeal.mergeRoutePlan) {
              GRProbeSelfHeal.mergeRoutePlan(plan, rev);
            }
          } catch (eMerge) {}
          return global.GRPackLoader.applyRoutePlan(plan, ctx).then(function (applied) {
            tick.dynamic_kicked = applied.kicked || [];
            tick.stop_probe = !!applied.stop_probe;
            ticks.push(tick);
            if (applied.stop_probe && tick.coverage_complete) {
              global.__GR_STOP_PROBE__ = true;
              try {
                global.__GR_MULTI_TICK_ACTIVE__ = null;
              } catch (eClr2) {}
              return ticks;
            }
            if (applied.kicked && applied.kicked.length) {
              emptyKicks = 0;
              // Yield after heavy kicks — longer after B10 so uploads finish before next plan.
              var kickedHeavy = (applied.kicked || []).some(function (id) {
                id = String(id || "");
                return (
                  /^R\d{2}/.test(id) ||
                  id.indexOf("spotcheck") >= 0 ||
                  id.indexOf("B10") === 0 ||
                  id.indexOf("B7") === 0
                );
              });
              var gap = kickedHeavy
                ? clientHasB10()
                  ? 450
                  : 200
                : clientHasB10()
                  ? 280
                  : 120;
              return sleep(gap).then(oneTick);
            }
            // No new kicks: wait uploads / next analyze rev — patience before stop.
            emptyKicks++;
            if (plan && plan.stop_probe && tick.coverage_complete) {
              global.__GR_STOP_PROBE__ = true;
              return ticks;
            }
            if (emptyKicks >= emptyPatience) {
              // Safety exit only; server may still accept gap-fill on next page open (sticky cycle).
              return ticks;
            }
            // Post-B10 empty ticks: back off harder (completeness via uploads, not poll storm).
            return sleep(clientHasB10() ? 900 : 350).then(oneTick);
          });
        })
        .catch(function (err) {
          ticks.push({ n: i, error: String(err && err.message ? err.message : err) });
          return ticks;
        });
    }

    return oneTick().then(
      function (t) {
        try {
          global.__GR_MULTI_TICK_ACTIVE__ = null;
        } catch (eDone) {}
        // multi_tick ends ≠ stop healing: keep Brain reconcile + collect watchdog alive.
        try {
          if (global.GRProbeSelfHeal) {
            if (!global.__GR_SELF_HEAL_ACTIVE__ && GRProbeSelfHeal.start) {
              GRProbeSelfHeal.start({ need_hard_anchor: !clientHasB10() });
            } else if (GRProbeSelfHeal.tick) {
              GRProbeSelfHeal.tick();
            }
            if (GRProbeSelfHeal.loopB) GRProbeSelfHeal.loopB(true);
          }
        } catch (eHealMt) {}
        return t;
      },
      function (err) {
        try {
          global.__GR_MULTI_TICK_ACTIVE__ = null;
        } catch (eErr) {}
        throw err;
      }
    );
  }

  /**
   * After multi-tick, keep applying route_plan until coverage terminal or timeout.
   * Improves mid landings when maxTicks exhausted mid-flight.
   */
  /**
   * Optional short-visit eager mid (A-PROBE-1 / norm/03–04).
   * Default OFF: dynamic packs only via route_plan.
   * When opts.eagerMid===true, only non-Soft-gated packs (never requires_soft_v2).
   */
  function eagerMidAndB8(ctx) {
    var Col = global.GRCollectors;
    if (!Col || !GRPackLoader) return Promise.resolve({ kicked: [] });
    // iss/22 P2: stamp bypass for server BattleLog / ops observability
    try {
      global.__GR_BYPASS__ = global.__GR_BYPASS__ || [];
      global.__GR_BYPASS__.push({
        kind: "eager_mid",
        ts: Date.now(),
        session_id: (ctx && ctx.session_id) || "",
      });
      if (ctx && ctx.queue && typeof ctx.queue.enqueue === "function") {
        ctx.queue.enqueue({
          session_id: ctx.session_id,
          batch_id: "B0_bootstrap",
          source: "main",
          priority: 40,
          force: false,
          payload: {
            fields: {
              brain_bypass: true,
              bypass_reason: "eager_mid",
              bypass_at_ms: Date.now(),
            },
          },
        });
      }
    } catch (eByp) {}
    var packs = [];
    // Soft-gated packs (B16/B13/B15/B6) MUST NOT eager-kick — brain Soft gate only.
    if (Col.defaultEagerMidPacks) packs = packs.concat(Col.defaultEagerMidPacks());
    else if (Col.defaultMidPacks) {
      // legacy: strip Soft-gated ids if old helper still used
      packs = packs.concat(
        (Col.defaultMidPacks() || []).filter(function (p) {
          var id = (p && (p.pack_id || p.id)) || "";
          return (
            id !== "B16_fast_signals" &&
            id !== "B13_authorized" &&
            id !== "B15_cross_curves" &&
            id !== "B6_risk"
          );
        })
      );
    }
    // Re-kick B8 if not already sent (gateway coverage).
    if (Col.get && Col.get("B8_gateway_early")) {
      var Q = global.GRUploadQueue;
      var sentB8 =
        Q &&
        Q.alreadySent &&
        Q.alreadySent({ session_id: ctx.session_id, batch_id: "B8_gateway", source: "gateway" });
      if (!sentB8) {
        var b8 = Col.get("B8_gateway_early");
        packs.push({
          id: "B8_gateway_early",
          pack_id: "B8_gateway_early",
          batch_id: "B8_gateway",
          source: "gateway",
          priority: 99,
          schedule: "static",
          run: function (c) {
            return b8.run(c);
          },
        });
      }
    }
    var kickedMap = (GRPackLoader.alreadyKicked && GRPackLoader.alreadyKicked()) || {};
    packs = packs.filter(function (p) {
      var id = p.id || p.pack_id;
      var bid = p.batch_id || id;
      var src = p.source || "main";
      if (kickedMap[id] || kickedMap[bid]) {
        // mid may have been sticky-kicked before upload — still allow if not sent
        var Q2 = global.GRUploadQueue;
        if (
          Q2 &&
          Q2.alreadySent &&
          Q2.alreadySent({ session_id: ctx.session_id, batch_id: bid, source: src })
        ) {
          return false;
        }
        // clear sticky for retry
        if (src === "main" || bid.indexOf("B1") === 0 || bid.indexOf("B10") === 0 || bid.indexOf("B13") === 0 || bid.indexOf("B15") === 0 || bid.indexOf("B16") === 0) {
          /* keep in list for mid retry */
        } else if (bid === "B8_gateway") {
          /* keep for B8 retry */
        } else {
          return false;
        }
      }
      var Q3 = global.GRUploadQueue;
      if (
        Q3 &&
        Q3.alreadySent &&
        Q3.alreadySent({ session_id: ctx.session_id, batch_id: bid, source: src })
      ) {
        return false;
      }
      return true;
    });
    if (!packs.length) return Promise.resolve({ kicked: [] });
    // Soft-clear mid sticky kicks so kickAll re-runs
    packs.forEach(function (p) {
      /* kickAll marks after success; softSkip handled in applyRoutePlan for route_plan path */
    });
    return GRPackLoader.kickAll(packs, ctx).then(function () {
      return {
        kicked: packs.map(function (p) {
          return p.id;
        }),
      };
    });
  }

  function coverageFollowup(ctx, maxMs) {
    maxMs = maxMs == null ? 20000 : maxMs;
    var start = Date.now();
    var rechecks = 0;
    function sleep(ms) {
      return new Promise(function (r) {
        setTimeout(r, ms);
      });
    }
    function loop() {
      if (global.__GR_STOP_PROBE__ || pageUnloading()) {
        return Promise.resolve({ stopped: true });
      }
      if (Date.now() - start > maxMs) {
        return Promise.resolve({ timeout: true, rechecks: rechecks });
      }
      // Periodic eager mid/B8 recheck independent of route_plan presence
      var eagerP =
        rechecks % 3 === 0
          ? eagerMidAndB8(ctx).catch(function () {
              return { kicked: [] };
            })
          : Promise.resolve({ kicked: [] });
      rechecks++;
      return eagerP
        .then(function () {
          return fetchLatestAnalysis(ctx.apiBase, ctx.session_id);
        })
        .then(function (result) {
          if (!result) return sleep(350).then(loop);
          var plan = result.route_plan || (result.brain && result.brain.route_plan);
          var cov = result.coverage || {};
          if ((plan && plan.stop_probe && cov.coverage_complete) || result.analysis_terminal) {
            global.__GR_STOP_PROBE__ = true;
            return { done: true, coverage_complete: true, rechecks: rechecks };
          }
          if (plan && plan.packs && plan.packs.length) {
            return global.GRPackLoader.applyRoutePlan(plan, ctx).then(function () {
              return waitUploadIdle(1000).then(function () {
                return sleep(180).then(loop);
              });
            });
          }
          return sleep(350).then(loop);
        })
        .catch(function () {
          return sleep(450).then(loop);
        });
    }
    return loop();
  }

  var Boot = {
    version: "0.4.0",
    /** Compat for V-08: queue a late batch flushed on pagehide. */
    enqueue: function (batchId, payload, opts) {
      opts = opts || {};
      var Q = global.GRUploadQueue;
      if (!Q) return 0;
      try {
        if (
          global.__GR_HALT_UPLOADS__ ||
          global.__GR_CYCLE_CLOSED__ ||
          (Q.isHalted && Q.isHalted(global.__GR_SESSION_ID__))
        ) {
          return 0;
        }
      } catch (eH) {}
      Q.enqueue({
        session_id: global.__GR_SESSION_ID__,
        batch_id: batchId,
        source: opts.source || "main",
        inject_path: global.__GR_INJECT_PATH__ || "app",
        priority: opts.priority || 50,
        payload: payload || {},
      });
      return Q.stats().pending;
    },
    pendingCount: function () {
      var Q = global.GRUploadQueue;
      return Q ? Q.stats().pending : 0;
    },
    flush: function (reason) {
      var Q = global.GRUploadQueue;
      return Q ? Q.flush(reason || "flush") : { flushed: 0 };
    },
    multiTickLoop: multiTickLoop,
    reconcileProbeStatus: reconcileProbeStatus,
    start: function (opts) {
      opts = opts || {};
      // FE cool cache timeout / version mismatch → clear and re-probe identity.
      try {
        var productVerBoot =
          (global.__GR_BOOT__ && (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
          global.__GR_PRODUCT_VERSION__ ||
          "";
        global.__GR_PRODUCT_VERSION__ = productVerBoot || global.__GR_PRODUCT_VERSION__;
        if (global.GRStorage && GRStorage.clearCoolIfExpired) {
          var cleared = GRStorage.clearCoolIfExpired(Date.now());
          if (cleared) {
            global.__GR_STOP_PROBE__ = false;
            global.__GR_SKIP_IDENTITY__ = false;
            global.__GR_HALT_UPLOADS__ = false;
            global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
            global.__GR_KICK_MS__.cool_cleared_expired = true;
          }
        }
        if (global.GRStorage && GRStorage.clearCoolIfVersionMismatch) {
          var clearedV = GRStorage.clearCoolIfVersionMismatch(productVerBoot);
          if (clearedV) {
            global.__GR_STOP_PROBE__ = false;
            global.__GR_SKIP_IDENTITY__ = false;
            global.__GR_HALT_UPLOADS__ = false;
            global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
            global.__GR_KICK_MS__.cool_cleared_version = true;
          }
        }
      } catch (eCool) {}
      // Single-flight: nginx + Next dual path + SPA remount must not mint two cycles.
      try {
        var pipeKey =
          "gr_pipe_" +
          String(
            (global.__GR_BOOT__ && (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
              global.__GR_PRODUCT_VERSION__ ||
              "dev"
          );
        if (sessionStorage.getItem(pipeKey) === "1" && global.__GR_PIPELINE_P__) {
          return global.__GR_PIPELINE_P__;
        }
        if (global.__GR_PIPELINE_STARTED__) {
          return global.__GR_PIPELINE_P__ || Promise.resolve({ skipped: true, reason: "single_flight" });
        }
        sessionStorage.setItem(pipeKey, "1");
      } catch (ePipe) {
        if (global.__GR_PIPELINE_STARTED__) {
          return global.__GR_PIPELINE_P__ || Promise.resolve({ skipped: true, reason: "single_flight" });
        }
      }
      global.__GR_PIPELINE_STARTED__ = true;
      var tBoot = Date.now();
      global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || { boot_start: tBoot };
      // --- v57 race: resolve endpoints + mint ids SYNCHRONOUSLY (no module wait) ---
      var apiBase = endpoint(opts);
      // Version self-heal vs BE (does not block pipeline if network fails).
      try {
        fetchSdkBootstrap(apiBase);
      } catch (eBs) {}
      var injectPath = resolveInjectPath(opts);
      var siteId = resolveSiteId(opts);
      var vt =
        opts.visitor_terminal_id ||
        mintVtInline();
      var cycleHint = resolveCycleHint(opts);
      global.__GR_INJECT_PATH__ = injectPath;
      global.__GR_API_BASE__ = apiBase;
      global.__GR_SESSION_ID__ = cycleHint;
      global.__GR_CYCLE_ID__ = cycleHint;
      global.__GR_CYCLE_HINT__ = cycleHint;
      global.__GR_VTID__ = vt;
      if (siteId) global.__GR_SITE_ID__ = siteId;

      var gwBaseEarly = gwEndpoint(opts, apiBase);
      // Never point "gateway base" at ingest-only host without a real gv derive —
      // fireEarlyB8 will still dual-path apiBase as CDN fallback.
      global.__GR_GW_BASE__ = gwBaseEarly || deriveGwBase(apiBase) || apiBase;

      // ---------- VERSION-SCOPED PROBE STATE (before any identity race) ----------
      // product_version is authority: mismatch auto-clears cool + cycle (no user cookie clear).
      var wantVer =
        productVerBoot ||
        (global.__GR_BOOT__ &&
          (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
        global.__GR_PRODUCT_VERSION__ ||
        "";
      var forceIdentityBoot = false;
      try {
        forceIdentityBoot =
          !!(opts && opts.force_identity) ||
          !!global.__GR_FORCE_IDENTITY__ ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.force_identity) ||
          /(?:^|[?&])gr_force=1(?:&|$)/.test(String(location.search || ""));
        if (forceIdentityBoot) {
          global.__GR_FORCE_IDENTITY__ = 1;
          if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
          if (global.GRStorage && GRStorage.clearCycleId) GRStorage.clearCycleId();
        }
      } catch (eFb) {}
      var localCool = false;
      try {
        if (global.GRStorage && GRStorage.ensureProbeStateForVersion) {
          var pst = GRStorage.ensureProbeStateForVersion(wantVer);
          localCool = !!pst.coolActive;
          if (pst.cycleId) {
            cycleHint = pst.cycleId;
            global.__GR_CYCLE_HINT__ = pst.cycleId;
            global.__GR_SESSION_ID__ = pst.cycleId;
            global.__GR_CYCLE_ID__ = pst.cycleId;
          }
          global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
          global.__GR_KICK_MS__.probe_state = pst;
        } else if (global.GRStorage && GRStorage.isCoolActive) {
          localCool = !!GRStorage.isCoolActive(Date.now(), wantVer);
        }
      } catch (eLc) {}
      if (!localCool && !(global.GRStorage && GRStorage.ensureProbeStateForVersion)) {
        try {
          var coolUntil = 0;
          var mCool = document.cookie.match(/(?:^|;\s*)gr_cool_until_v1=([^;]+)/);
          if (mCool && mCool[1]) coolUntil = parseInt(decodeURIComponent(mCool[1]), 10) || 0;
          if (!coolUntil) {
            try {
              coolUntil = parseInt(localStorage.getItem("gr_probe_cool_until_v1") || "0", 10) || 0;
            } catch (eLs) {}
          }
          var coolVer = "";
          try {
            var mVer = document.cookie.match(/(?:^|;\s*)gr_product_version_v1=([^;]+)/);
            if (mVer && mVer[1]) coolVer = decodeURIComponent(mVer[1]);
            if (!coolVer) coolVer = localStorage.getItem("gr_product_version_v1") || "";
          } catch (eVer) {}
          localCool =
            coolUntil > Date.now() &&
            (!wantVer || (!!coolVer && coolVer === wantVer));
          // Version mismatch without storage module: remint cycle so we never hit complete bag.
          if (!localCool && wantVer && coolVer && coolVer !== wantVer) {
            cycleHint = mintCycleId();
            writeStickyCookie(cycleCookieName(), cycleHint, 86400);
            expireCookieEverywhere("gr_cycle_v1");
            expireCookieEverywhere("_g5_c");
            writeStickyCookie("gr_cycle_product_version_v1", wantVer, 86400);
            global.__GR_CYCLE_HINT__ = cycleHint;
            global.__GR_SESSION_ID__ = cycleHint;
          }
        } catch (eCk) {}
      }
      // Local cool is a HINT only. Open must re-validate schedule final + silicon.
      // If server says active / need_hard_anchor / force_identity, fall through to full probe.
      if (localCool && !forceIdentityBoot && opts.force_identity !== true) {
        global.__GR_KICK_MS__.local_cool_pending = true;
        var coolCookies = collectCookieFields();
        var coolTok = embedTokenFromUrl();
        var coolOpenBody = {
          visitor_terminal_id: vt,
          session_id: cycleHint,
          inject_path: injectPath,
          site_id: siteId || undefined,
          embed_token: coolTok || undefined,
          cookie_fields: Object.keys(coolCookies).length ? coolCookies : undefined,
          meta: {
            fe: "gr.boot",
            cool_local: true,
            href: String(typeof location !== "undefined" ? location.href : ""),
            inject_path: injectPath,
            site_id: siteId || undefined,
            cookie_fields: Object.keys(coolCookies).length ? coolCookies : undefined,
            product_version:
              productVerBoot ||
              (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
              undefined,
            version:
              productVerBoot ||
              (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
              undefined,
          },
        };
        var coolP = postJson(apiBase + "/v1/session/open", coolOpenBody, { priority: "high" })
          .then(function (opened) {
            opened = opened || {};
            try {
              applyOpenEntitlement(opened);
            } catch (eEntC) {}
            var cpsC = opened.cycle_probe_status || {};
            var scheduleFinal =
              cpsC.brain_schedule_final === true ||
              cpsC.final_analysis_ok === true ||
              cpsC.cycle_status === "complete" ||
              opened.cycle_status === "complete" ||
              (cpsC.coverage_complete === true &&
                (cpsC.probe_complete === true || cpsC.analysis_terminal === true));
            var serverSkip =
              opened.skip_identity_probe === true ||
              opened.skip_session_probe === true ||
              opened.business_state === "identity_complete_cool" ||
              opened.phase === "cool";
            var forceReprobe =
              !!opened.force_identity_probe ||
              !!opened.client_hint_superseded ||
              !!opened.need_hard_anchor ||
              opened.phase === "active" ||
              opened.phase === "probing" ||
              opened.skip_identity_probe === false ||
              (opened.reprobe_reason && String(opened.reprobe_reason).length);
            // Thin cool: no schedule final → must re-enter probe race.
            if (forceReprobe || (serverSkip && !scheduleFinal) || (!serverSkip && !scheduleFinal)) {
              try {
                global.__GR_KICK_MS__.local_cool_invalidated = {
                  force: !!forceReprobe,
                  schedule_final: !!scheduleFinal,
                  phase: opened.phase || null,
                  need_hard: !!opened.need_hard_anchor,
                };
                if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
                global.__GR_STOP_PROBE__ = false;
                global.__GR_SKIP_IDENTITY__ = false;
                global.__GR_HALT_UPLOADS__ = false;
                global.__GR_PHASE__ = opened.phase || "active";
                if (opened.session_id || opened.cycle_id) {
                  var cs0 = String(opened.session_id || opened.cycle_id);
                  cycleHint = cs0;
                  global.__GR_SESSION_ID__ = cs0;
                  global.__GR_CYCLE_ID__ = cs0;
                  global.__GR_CYCLE_HINT__ = cs0;
                  if (global.GRStorage && GRStorage.setCycleId) {
                    GRStorage.setCycleId(
                      cs0,
                      opened.product_version || productVerBoot || undefined
                    );
                  }
                }
              } catch (eInv) {}
              // Restart once with force_identity so we skip localCool short-circuit.
              global.__GR_PIPELINE_STARTED__ = false;
              if (opts && opts.__from_cool_invalidate) {
                // Should not re-enter; treat as open active for caller.
                return {
                  cool: false,
                  skip_identity_probe: false,
                  force_identity: true,
                  session_id: global.__GR_SESSION_ID__ || cycleHint,
                  opened: opened,
                };
              }
              return Boot.start(
                Object.assign({}, opts, {
                  force_identity: true,
                  __from_cool_invalidate: true,
                })
              );
            }
            // Confirmed schedule-final cool: halt + show last result.
            global.__GR_STOP_PROBE__ = true;
            global.__GR_SKIP_IDENTITY__ = true;
            global.__GR_HALT_UPLOADS__ = true;
            global.__GR_PHASE__ = "cool";
            global.__GR_KICK_MS__.local_cool_skip = true;
            global.__GR_BRAIN_SCHEDULE_FINAL__ = true;
            try {
              var Qc = global.GRUploadQueue;
              if (Qc && Qc.halt) Qc.halt("local_cool", cycleHint, "cool");
            } catch (eH) {}
            if (opened.cool_until_ms && global.GRStorage && GRStorage.setCoolUntil) {
              GRStorage.setCoolUntil(
                opened.cool_until_ms,
                opened.product_version ||
                  productVerBoot ||
                  (global.__GR_BOOT__ && global.__GR_BOOT__.version)
              );
            }
            if (opened.session_id || opened.cycle_id) {
              var cs = String(opened.session_id || opened.cycle_id);
              global.__GR_SESSION_ID__ = cs;
              global.__GR_CYCLE_ID__ = cs;
              if (global.GRStorage && GRStorage.setCycleId) {
                GRStorage.setCycleId(
                  cs,
                  opened.product_version || productVerBoot || undefined
                );
              }
            }
            var lir =
              opened.last_identity_result ||
              (opened.session && opened.session.last_identity_result) ||
              null;
            if (lir) {
              global.__GR_LAST_ANALYZE__ = {
                result: lir,
                route_plan: lir.route_plan || (lir.brain && lir.brain.route_plan) || null,
              };
            }
            if (opened.cycle_probe_status) {
              global.__GR_CYCLE_PROBE_STATUS__ = opened.cycle_probe_status;
            }
            if (opened.business_state) {
              global.__GR_BUSINESS_STATE__ = opened.business_state;
            }
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.markCool) {
                GRProbeLifecycle.markCool("local_cool_confirmed");
              }
            } catch (eLc2) {}
            return {
              cool: true,
              local_cool: true,
              skip_identity_probe: true,
              session_id: global.__GR_SESSION_ID__,
              opened: opened,
              last_identity_result: lir,
              brain_schedule_final: true,
            };
          })
          .catch(function (e) {
            // Open failed while local cool: do NOT freeze forever — re-probe once.
            try {
              if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
              global.__GR_STOP_PROBE__ = false;
              global.__GR_SKIP_IDENTITY__ = false;
              global.__GR_HALT_UPLOADS__ = false;
              global.__GR_PIPELINE_STARTED__ = false;
              global.__GR_KICK_MS__.local_cool_open_err = String(e && e.message ? e.message : e);
            } catch (eE) {}
            if (opts && (opts.__from_cool_open_err || opts.__from_cool_invalidate)) {
              return {
                cool: false,
                open_err: String(e && e.message ? e.message : e),
                skip_identity_probe: false,
              };
            }
            return Boot.start(
              Object.assign({}, opts, { force_identity: true, __from_cool_open_err: true })
            );
          });
        global.__GR_PIPELINE_P__ = coolP;
        if (typeof opts.onDone === "function") {
          coolP.then(function (r) {
            try {
              opts.onDone(r);
            } catch (eD) {}
          });
        }
        return coolP;
      }

      // Not cool: allow probe race
      global.__GR_STOP_PROBE__ = false;
      global.__GR_HALT_UPLOADS__ = false;

      // Fire open + B8 + race in parallel — same client-minted cycle_id (B8↔FE join).
      var openCookies = collectCookieFields();
      var openTok = embedTokenFromUrl();
      var openBody = {
        visitor_terminal_id: vt,
        session_id: cycleHint,
        inject_path: injectPath,
        site_id: siteId || undefined,
        embed_token: openTok || undefined,
        cookie_fields: Object.keys(openCookies).length ? openCookies : undefined,
        force_identity: forceIdentityBoot || undefined,
        storage_bind: (function () {
          try {
            return global.GRStorage && GRStorage.getStorageBind
              ? GRStorage.getStorageBind() || undefined
              : undefined;
          } catch (eSb) {
            return undefined;
          }
        })(),
        meta: {
          fe: "gr.boot",
          race: "l1",
          boot_fast: true,
          href: String(typeof location !== "undefined" ? location.href : ""),
          inject_path: injectPath,
          identity_class: "js",
          site_id: siteId || undefined,
          cookie_fields: Object.keys(openCookies).length ? openCookies : undefined,
          product_version:
            productVerBoot ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
            undefined,
          version:
            productVerBoot ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
            undefined,
          force_identity: forceIdentityBoot || undefined,
        },
      };
      // B8 ASAP with client mint — do not wait open (gateway open_cycle honors hint).
      fireEarlyB8(apiBase, gwBaseEarly || apiBase, cycleHint, siteId, injectPath, vt).then(
        function (b8) {
          if (earlyB8Landed(b8)) {
            global.__GR_EARLY_B8__ = b8;
            global.__GR_KICK_MS__.b8_ms = Date.now() - tBoot;
            global.__GR_KICK_MS__.b8_via = b8.__b8_via || null;
          } else {
            global.__GR_KICK_MS__.b8_first_miss = true;
          }
          // If gateway rewrote sid (should not), keep FE mint for main ingest.
          try {
            if (b8 && b8.session_id && b8.session_id !== cycleHint) {
              global.__GR_KICK_MS__.b8_sid_mismatch = {
                client: cycleHint,
                gateway: b8.session_id,
              };
            }
          } catch (eM) {}
        }
      );
      // Soft B8 retries while on page — same cycleHint; require landed batch not cool-skip.
      // Longer tail (5s/10s) covers slow CDN + late-openable origin gv.
      [80, 250, 700, 1600, 3500, 8000].forEach(function (ms) {
        setTimeout(function () {
          if (pageUnloading()) return;
          // Allow B8 even after cool STOP_PROBE — protocol join is still valuable on page.
          if (earlyB8Landed(global.__GR_EARLY_B8__)) return;
          fireEarlyB8(
            apiBase,
            global.__GR_GW_BASE__ || gwBaseEarly,
            global.__GR_SESSION_ID__ || cycleHint,
            siteId,
            injectPath,
            global.__GR_VTID__ || vt
          ).then(function (b8) {
            if (earlyB8Landed(b8)) {
              global.__GR_EARLY_B8__ = b8;
              global.__GR_KICK_MS__.b8_retry_landed_ms = Date.now() - tBoot;
              global.__GR_KICK_MS__.b8_via = b8.__b8_via || null;
            }
          });
        }, ms);
      });
      // Seal ready barrier: uploads wait for grant (prod sealed) without full open gate.
      var sealReadyResolve = null;
      try {
        if (!global.__GR_SEAL_READY_P__) {
          global.__GR_SEAL_READY_P__ = new Promise(function (resolve) {
            sealReadyResolve = resolve;
          });
          // Fail-open after 18s so short visits still progress (lab/offline);
          // longer than before: prod open+CF lag was causing seal_grant_unavailable.
          setTimeout(function () {
            try {
              if (sealReadyResolve) sealReadyResolve(false);
            } catch (eT) {}
          }, 18000);
        }
      } catch (eSr) {}
      function applySealFromOpen(opened) {
        try {
          var needSeal =
            (opened && opened.require_sealed_ingest === true) ||
            (opened && opened.policy && opened.policy.require_sealed_ingest === true) ||
            !!global.__GR_REQUIRE_SEALED__ ||
            !!global.__GR_SEEN_SEALED_REQUIRED__;
          if (needSeal) {
            global.__GR_REQUIRE_SEALED__ = true;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.require_sealed_ingest = true;
          }
          if (opened && opened.seal_grant) {
            global.__GR_SEAL_GRANT__ = opened.seal_grant;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.seal_grant = opened.seal_grant;
            if (global.GRSeal) {
              if (needSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
              if (GRSeal.setGrant) GRSeal.setGrant(opened.seal_grant);
              if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            }
          }
          if (sealReadyResolve) {
            sealReadyResolve(!!(opened && opened.seal_grant));
            sealReadyResolve = null;
          }
          try {
            if (global.GRUploadQueue && GRUploadQueue.kick) GRUploadQueue.kick();
          } catch (eK) {}
        } catch (eSeal0) {}
      }
      // Early prod flag from inject (before bootstrap/open).
      try {
        if (global.__GR_REQUIRE_SEALED__ || (global.__GR_BOOT__ && global.__GR_BOOT__.require_sealed_ingest)) {
          if (global.GRSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
        }
      } catch (eEarly) {}
      var openP = postJson(apiBase + "/v1/session/open", openBody, {
        priority: "high",
        keepalive: true,
      })
        .then(function (opened) {
          global.__GR_KICK_MS__.open_ms = Date.now() - tBoot;
          global.__GR_OPEN_RESULT__ = opened;
          // CRITICAL: seal grant ASAP (before B0 upload) — primary 426 fix.
          applySealFromOpen(opened || {});
          try {
            if (opened && opened.debug === true) global.__GR_DEBUG__ = true;
            if (opened && opened.storage_bind && global.GRStorage && GRStorage.setStorageBind) {
              var sb =
                typeof opened.storage_bind === "string"
                  ? opened.storage_bind
                  : opened.storage_bind.bind || "";
              if (sb) GRStorage.setStorageBind(sb);
            }
            // Admin panel FE retry/timeout knobs (open.policy.fe_retry)
            var pol = opened && opened.policy;
            var feRetry = pol && (pol.fe_retry || pol.feRetry);
            if (feRetry && global.GRProbeLifecycle && GRProbeLifecycle.applyServerPolicy) {
              GRProbeLifecycle.applyServerPolicy(feRetry);
            }
            if (opened && opened.config_version != null) {
              global.__GR_CONFIG_VERSION__ = opened.config_version;
            }
          } catch (eBind) {}
          // Re-kick B8 with confirmed vt after open (same cycleHint)
          fireEarlyB8(
            apiBase,
            global.__GR_GW_BASE__,
            cycleHint,
            siteId,
            injectPath,
            opened.visitor_terminal_id || vt
          ).then(function (b8) {
            if (earlyB8Landed(b8)) {
              global.__GR_EARLY_B8__ = b8;
              global.__GR_KICK_MS__.b8_ms = Date.now() - tBoot;
              global.__GR_KICK_MS__.b8_via = b8.__b8_via || null;
            }
          });
          return opened;
        })
        .catch(function (e) {
          global.__GR_KICK_MS__.open_err = String(e && e.message ? e.message : e);
          try {
            if (sealReadyResolve) {
              sealReadyResolve(false);
              sealReadyResolve = null;
            }
          } catch (eSr2) {}
          // Still allow race: ingest auto-opens session
          return { ok: false, session_id: cycleHint, visitor_terminal_id: vt, visitor_facet: "browser" };
        });

      // L1 race modules only (queue+l1) — NO full registry before first upload (v57).
      var raceModP = ensureRaceModules().then(function (how) {
        global.__GR_KICK_MS__.race_modules_ms = Date.now() - tBoot;
        global.__GR_KICK_MS__.race_load = how || "ok";
      });
      // Rest (registry) + full (sandbox) start after L1 B0 enqueued (pipeline below).
      var restModP = null;
      var fullModP = null;

      function sleep(ms) {
        return new Promise(function (resolve) {
          setTimeout(resolve, ms);
        });
      }

      // v57: L1 ready → B0 first; open may still be in flight.
      var pipelineP = raceModP.then(function () {
        if (global.GRPackLoader && GRPackLoader.resetKicked) GRPackLoader.resetKicked();
        var Q = global.GRUploadQueue;
        if (!Q || !global.GRL1) {
          throw new Error("race_modules_missing_queue_or_l1");
        }
        // Short-visit stream: higher start concurrency; still priority-sort B0 first.
        // Overrides from open.policy.fe_retry when panel published.
        var fePol =
          (global.GRProbeLifecycle && global.GRProbeLifecycle.POLICY) || {};
        Q.configure({
          apiBase: apiBase,
          inject_path: injectPath,
          session_id: cycleHint,
          // iss/70 P2: start low; AIMD + effectiveUploadCap raise within max.
          concurrency:
            opts.concurrency != null
              ? opts.concurrency
              : fePol.upload_concurrency != null
                ? fePol.upload_concurrency
                : 3,
          concurrency_max:
            opts.concurrency_max != null
              ? opts.concurrency_max
              : fePol.upload_concurrency_max != null
                ? fePol.upload_concurrency_max
                : 6,
          ramp_after_first:
            opts.ramp_after_first ||
            (fePol.upload_ramp_after != null ? fePol.upload_ramp_after : 4),
          mid_ramp_concurrency:
            opts.mid_ramp_concurrency ||
            (fePol.upload_mid_ramp != null ? fePol.upload_mid_ramp : 4),
          alive_retry_ms:
            fePol.client_alive_retry_ms != null
              ? fePol.client_alive_retry_ms
              : 30000,
        });
        bindHide(Q);
        // Registry deferred until L1 rest enqueued — frees bandwidth for B0+rest upload first.

        var sessionId = cycleHint;
        global.__GR_SESSION_ID__ = sessionId;
        global.__GR_CYCLE_ID__ = sessionId;

        var gwBase = gwEndpoint(opts, apiBase);
        global.__GR_GW_BASE__ = gwBase;
        var pageId =
          (opts && opts.page_id) ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.page_id) ||
          global.__GR_PAGE_ID__ ||
          (function () {
            try {
              if (typeof location === "undefined") return null;
              var host = String(location.hostname || "");
              var path = String(location.pathname || "/");
              return host ? host + path : path || null;
            } catch (e) {
              return null;
            }
          })();
        if (pageId) global.__GR_PAGE_ID__ = pageId;
        var ctx = {
          apiBase: apiBase,
          gwBase: gwBase,
          session_id: sessionId,
          inject_path: injectPath,
          page_id: pageId,
          fields: { page_id: pageId },
          queue: Q,
          // PV first-party SDK asset — Standard C content-hash nest_frame when known.
          nestUrl: (function () {
            try {
              var man =
                global.__GR_MANIFEST__ ||
                (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
                null;
              if (man && man.assets && man.assets.nest_frame) {
                return String(man.assets.nest_frame);
              }
            } catch (eNu) {}
            // Opaque-only: never invent nest_frame.html on the wire.
            return "";
          })(),
        };

        var L1 = global.GRL1;
        var l1Plan = (L1.progressivePlan && L1.progressivePlan()) || {
          first: "B0_bootstrap",
          rest: ["B1_conflict", "B12_anti_camouflage", "B2_hardware", "B3_system"],
          deferred_heavy: ["B10_hw_curves", "B7_sandbox", "B11_interaction"],
        };
        var wave1 = [l1Plan.first].concat(l1Plan.rest || []);
        var wave2 = l1Plan.deferred_heavy || ["B10_hw_curves", "B7_sandbox"];

        var t0 = Date.now();
        global.__GR_KICK_MS__ = Object.assign(global.__GR_KICK_MS__ || {}, {
          start: t0,
          boot_start: tBoot,
          wave1: wave1.slice(),
          wave2: wave2.slice(),
          race_no_open_gate: true,
          l1_first: true,
          requires_registry_for_b0: false,
        });

        // --- B0 FIRST (no registry), then parallel rest collect while B0 uploads ---
        // Sealed transport is an upload concern, not a collection gate.
        // Collectors freeze captures in memory at enqueue; upload_queue waits
        // for the grant and never falls back to plain ingest.
        var restP = null;
        var kick1 = Promise.resolve().then(function () {
          // Start B0 and the independent L1 rest in the same turn. B0 keeps
          // the highest queue priority; it no longer serializes collection.
          var b0P = L1.runB0(ctx);
          try {
            restP = L1.runProgressiveRest(ctx);
          } catch (eRestKick) {
            restP = Promise.resolve([]);
          }
          // Front-load the hard registry fetch while L1 collectors run.
          try {
            ensureStaticHardModules().then(function () {
              global.__GR_KICK_MS__.hard_early_ms = Date.now() - tBoot;
            });
          } catch (eHardEarly) {}
          return b0P;
        }).then(function (b0) {
          global.__GR_KICK_MS__.kick1_at = Date.now() - tBoot;
          global.__GR_KICK_MS__.b0_enqueued = true;
          // Soft-ramp now: B0 is first in queue (prio 120); open more slots for parallel rest.
          try {
            if (Q.softRamp) Q.softRamp(opts.mid_ramp_concurrency || 8);
            else Q.configure({ concurrency: Math.max(cfgConcurrency(), 8) });
          } catch (eC) {}
          // B11 ASAP after B0 (RPA) — only result policy may disable it.
          try {
            if (rpaEnabledForPlan()) bindEarlyB11(ctx);
          } catch (eB11) {}
          // Start rest modules early so rpa_monitor can upgrade B11 bind.
          try {
            if (rpaEnabledForPlan()) {
              ensureRestModules().then(function () {
                if (!global.__GR_RPA_BOUND__ || global.__GR_KICK_MS__.b11_early === "inline") {
                  if (global.GRRpaMonitor && global.__GR_KICK_MS__.b11_early === "inline") {
                    try {
                      global.__GR_RPA_BOUND__ = false;
                      bindEarlyB11(ctx);
                    } catch (eUp) {}
                  }
                }
              });
            }
          } catch (eRest) {}
          if (global.__GR_STOP_PROBE__ || pageUnloading()) {
            return { b0: b0, rest: [] };
          }
          global.__GR_KICK_MS__.l1_rest_at = Date.now() - tBoot;
          // L1 rest already started alongside B0. Process its completion
          // asynchronously so kick2/B10 is not held behind low-cost fields.
          (restP || Promise.resolve([])).then(function (rest) {
            global.__GR_KICK_MS__.l1_rest = (rest || []).map(function (r) {
              return r && r.batch_id;
            });
            // Re-fire B8 after first lite wave (same cycle) — nginx primary, no CF worker needed.
            fireEarlyB8(
              apiBase,
              global.__GR_GW_BASE__ || gwBase,
              cycleHint,
              siteId,
              injectPath,
              global.__GR_VTID__ || vt
            ).then(function (b8) {
              if (earlyB8Landed(b8)) {
                global.__GR_EARLY_B8__ = b8;
                global.__GR_KICK_MS__.b8_retry_ms = Date.now() - tBoot;
                global.__GR_KICK_MS__.b8_via = b8.__b8_via || null;
              }
            });
            // Defer static registry until B0 upload-ok OR 280ms — frees CDN for L1 stream.
            // Mid/dynamic registry loads only after rest + idle (on-demand split).
            restModP = waitFirstUploadOrTimeout(280).then(function (why) {
              global.__GR_KICK_MS__.registry_load_why = why;
              return ensureRestModules().then(function () {
                global.__GR_KICK_MS__.rest_modules_ms = Date.now() - tBoot;
              });
            });
            fullModP = restModP.then(function () {
              return whenIdleOrTimeout(350).then(function () {
                return ensureFullModules().then(function () {
                  global.__GR_KICK_MS__.full_modules_ms = Date.now() - tBoot;
                });
              });
            });
          }).catch(function () {});
          return { b0: b0, rest: [] };
        });
        function waitFirstUploadOrTimeout(ms) {
          return new Promise(function (resolve) {
            var done = false;
            function finish(why) {
              if (done) return;
              done = true;
              try {
                global.removeEventListener("gr-upload-ok", onOk);
              } catch (e) {}
              resolve(why || "timeout");
            }
            function onOk(ev) {
              var bid = ev && ev.detail && ev.detail.batch_id;
              if (bid === "B0_bootstrap" || bid === "B1_conflict") finish("upload_ok_" + bid);
            }
            try {
              global.addEventListener("gr-upload-ok", onOk);
            } catch (eA) {}
            setTimeout(function () {
              finish("timeout_" + ms);
            }, ms == null ? 280 : ms);
          });
        }
        function whenIdleOrTimeout(ms) {
          return new Promise(function (resolve) {
            var t = setTimeout(function () {
              resolve("timeout");
            }, ms == null ? 400 : ms);
            try {
              if (typeof requestIdleCallback === "function") {
                requestIdleCallback(
                  function () {
                    clearTimeout(t);
                    resolve("idle");
                  },
                  { timeout: ms == null ? 400 : ms }
                );
                return;
              }
            } catch (eI) {}
          });
        }
        function cfgConcurrency() {
          try {
            return (Q.stats && Q.stats().inflight) || 3;
          } catch (e) {
            return 3;
          }
        }
        global.__GR_KICK_MS__.kick1_started = Date.now() - tBoot;

        function filterPacks(list) {
          if (!global.GRPackLoader) return list || [];
          var kickedMap = (GRPackLoader.alreadyKicked && GRPackLoader.alreadyKicked()) || {};
          var l1k = global.__GR_L1_KICKED__ || {};
          return (list || []).filter(function (p) {
            var id = p.id || p.pack_id;
            var bid = p.batch_id || id;
            if (kickedMap[id] || kickedMap[bid] || l1k[bid] || l1k[id]) return false;
            if (
              Q.alreadySent &&
              Q.alreadySent({ session_id: sessionId, batch_id: bid, source: p.source || "main" })
            ) {
              return false;
            }
            return true;
          });
        }

        // Wave2 heavy: need static.hard (B10/B7) + sandbox; not mid/dense.
        // Await pack content match BEFORE B10 kick (prevents sticky v1 with product 124).
        var kick2 = kick1
          .then(function () {
            var sv =
              scriptVersion() ||
              global.__GR_SERVER_PRODUCT_VERSION__ ||
              global.__GR_PRODUCT_VERSION__ ||
              "";
            // Up to 2 content-gate attempts before wave2 (P2).
            function tryPacks(n) {
              return ensurePacksMatchServerVersion(String(sv || ""))
                .catch(function () {
                  return false;
                })
                .then(function (ok) {
                  if (ok || n <= 1) return !!ok;
                  return new Promise(function (r) {
                    setTimeout(function () {
                      r(tryPacks(n - 1));
                    }, 200);
                  });
                });
            }
            return tryPacks(2).then(function (ok) {
              try {
                global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
                global.__GR_KICK_MS__.packs_content_ok = !!ok;
                global.__GR_KICK_MS__.lite_algo = liteAlgoId();
              } catch (eK) {}
              return ok;
            });
          })
          .then(function (packsOk) {
            // Short-visit fast lane: B10/B7/B11 only need the hard registry
            // and the L1 collectors. Full modules (sandbox/deep/RPA support)
            // continue in the background and must not delay the first hard
            // silicon anchor.
            return ensureStaticHardModules().then(function () {
              return packsOk;
            });
          })
          .then(function (packsOk) {
            if (global.__GR_STOP_PROBE__ || pageUnloading()) return [];
            if (!global.GRPackLoader || !global.GRCollectors) return [];
            var Col = global.GRCollectors;
            try {
              global.__GR_WAVE2_PACKS_OK__ = !!packsOk;
            } catch (eW) {}
            // IMPORTANT: [] is truthy in JS — never use `wave2() || fallback`.
            var heavy = [];
            try {
              if (typeof Col.defaultStaticWave2 === "function") {
                heavy = Col.defaultStaticWave2() || [];
              }
            } catch (eW2) {
              heavy = [];
            }
            if (!heavy.length) {
              // Resolve from registry when wave2 empty (hard not ready / empty array bug).
              var ids = ["B10_hw_curves", "B7_sandbox"];
              for (var wi = 0; wi < ids.length; wi++) {
                var pid = ids[wi];
                var def = Col.get && Col.get(pid);
                if (def && typeof def.run === "function") {
                  heavy.push({
                    id: pid,
                    pack_id: pid,
                    batch_id: pid,
                    priority: def.priority || (pid.indexOf("B10") === 0 ? 91 : 60),
                    schedule: def.schedule || "static",
                    run: function (d) {
                      return function (c) {
                        return d.run(c);
                      };
                    }(def),
                  });
                }
              }
            }
            // Drop stubs without run (would mark kicked as noop and block retries).
            heavy = (heavy || []).filter(function (p) {
              return p && typeof p.run === "function";
            });
            // P2: if packs content gate failed, do NOT kick B10 yet (hard SLA will retry after reload).
            // Prevents product_version=new + cpu_loop_algo=v1 uploads from sticky lite.
            if (!packsOk) {
              heavy = heavy.filter(function (p) {
                var id = String(p.id || p.pack_id || p.batch_id || "");
                return id !== "B10_hw_curves" && id.indexOf("B10x_") !== 0;
              });
              try {
                if (global.GROps && GROps.report) {
                  // Lifecycle: waiting for pack reload — not a program fault.
                  GROps.report(
                    "wave2_defer_b10",
                    "kick",
                    { reason: "packs_content_not_ok", algo: liteAlgoId() },
                    "info"
                  );
                }
              } catch (eDef) {}
            }
            try {
              if (Col.get && Col.get("B11_interaction") && typeof Col.get("B11_interaction").run === "function") {
                var b11 = Col.get("B11_interaction");
                heavy = heavy.concat([
                  {
                    id: "B11_interaction",
                    pack_id: "B11_interaction",
                    batch_id: "B11_interaction",
                    priority: 80,
                    source: "main",
                    run: function (c) {
                      return b11.run(c);
                    },
                  },
                ]);
              }
            } catch (eH) {}
            heavy = filterPacks(heavy);
            global.__GR_KICK_MS__.kick2_at = Date.now() - tBoot;
            global.__GR_KICK_MS__.wave2 = heavy.map(function (p) {
              return p.id || p.pack_id;
            });
            global.__GR_KICK_MS__.hard_ready = hardPacksReady();
            global.__GR_KICK_MS__.wave2_has_b10 = heavy.some(function (p) {
              return String(p.id || p.pack_id || "") === "B10_hw_curves";
            });
            // Product: never concurrent hardware thrash — B10 then B7 then light (B11).
            function kickWave2Heavy(list, depth) {
              depth = depth || 0;
              list = filterPacks(list || []);
              if (!list.length) {
                if (depth >= 1) {
                  // iss/62 P1 (wave2_empty): distinguish a true empty (hard ready
                  // but B10 never kicked/sent — real scheduling hole) from benign
                  // re-entry (B10 already kicked/sent by an earlier wave2, the SLA
                  // path, or a route plan — the retry list filters to empty by
                  // design). Only the true case is a warn.
                  var b10Kicked = false;
                  var b10Sent = false;
                  try {
                    var km2 =
                      (global.GRPackLoader &&
                        GRPackLoader.alreadyKicked &&
                        GRPackLoader.alreadyKicked()) ||
                      {};
                    var l1k2 = global.__GR_L1_KICKED__ || {};
                    b10Kicked = !!(km2["B10_hw_curves"] || l1k2["B10_hw_curves"]);
                  } catch (eK2) {}
                  try {
                    if (Q && Q.alreadySent) {
                      b10Sent = !!Q.alreadySent({
                        session_id: sessionId,
                        batch_id: "B10_hw_curves",
                        source: "main",
                      });
                    }
                    var rec2 = global.__GR_RECEIVED_BATCHES__ || [];
                    for (var ri2 = 0; ri2 < rec2.length; ri2++) {
                      if (String(rec2[ri2].batch_id || "") === "B10_hw_curves") {
                        b10Sent = true;
                        break;
                      }
                    }
                  } catch (eS2) {}
                  try {
                    if (global.GROps) {
                      if (b10Kicked || b10Sent) {
                        if (GROps.dlog) {
                          GROps.dlog("wave2_empty_benign", {
                            b10_kicked: b10Kicked,
                            b10_sent: b10Sent,
                          });
                        }
                      } else {
                        GROps.wave2Empty({
                          hard_ready: !!hardPacksReady(),
                          retried: true,
                          b10_kicked: false,
                          b10_sent: false,
                        });
                      }
                    }
                  } catch (eOp0) {}
                  return Promise.resolve([]);
                }
                // Hard module not ready yet: retry load then rebuild pack list once.
                return ensureStaticHardModules()
                  .then(function () {
                    return new Promise(function (r) {
                      setTimeout(r, 120);
                    });
                  })
                  .then(function () {
                    return ensureStaticHardModules();
                  })
                  .then(function () {
                    var retry = [];
                    var Col2 = global.GRCollectors;
                    ["B10_hw_curves", "B7_sandbox"].forEach(function (pid) {
                      var def = Col2 && Col2.get && Col2.get(pid);
                      if (def && typeof def.run === "function") {
                        retry.push({
                          id: pid,
                          pack_id: pid,
                          batch_id: pid,
                          priority: def.priority || 91,
                          schedule: "static",
                          run: function (d) {
                            return function (c) {
                              return d.run(c);
                            };
                          }(def),
                        });
                      }
                    });
                    return kickWave2Heavy(retry, depth + 1);
                  });
              }
              var hw = [];
              var light = [];
              list.forEach(function (p) {
                if (GRPackLoader.isHardwarePack && GRPackLoader.isHardwarePack(p)) hw.push(p);
                else light.push(p);
              });
              hw.sort(function (a, b) {
                var ia = String(a.id || a.pack_id || "");
                var ib = String(b.id || b.pack_id || "");
                var rank = function (id) {
                  if (id === "B10_hw_curves") return 0;
                  if (id === "B2_hardware") return 1;
                  if (id === "B7_sandbox") return 2;
                  return 3;
                };
                return rank(ia) - rank(ib);
              });
              var chain = Promise.resolve([]);
              hw.forEach(function (p) {
                chain = chain.then(function (acc) {
                  return GRPackLoader.kickAll([p], ctx).then(function (r) {
                    return (acc || []).concat(r || []);
                  });
                });
              });
              if (light.length) {
                chain = chain.then(function (acc) {
                  return GRPackLoader.kickAll(light, ctx).then(function (r) {
                    return (acc || []).concat(r || []);
                  });
                });
              }
              return chain;
            }
            return kickWave2Heavy(heavy);
          })
          .catch(function (e2) {
            try {
              try { if (global.GROps) GROps.report("wave2_kick_fail", "kick", { err: String(e2 && e2.message || "") }, "error"); } catch (eOp) {}
            } catch (e3) {}
            return [];
          });

        // Hard-anchor SLA: if B10 never lands, re-load hard packs + re-kick (no user cache clear).
        // Root cause of WebKit sticky thin: wave2 missed once, cool/sticky prevented retry.
        // Also re-run when tab becomes visible again (background must not cancel SLA).
        (function scheduleHardAnchorSla() {
          // Prefer panel/open policy; fewer / slower ticks to cut hard_sla_retry storms.
          var polSla =
            (global.GRProbeLifecycle && global.GRProbeLifecycle.POLICY) || {};
          var delays =
            Array.isArray(polSla.hard_sla_delays_ms) && polSla.hard_sla_delays_ms.length
              ? polSla.hard_sla_delays_ms.slice()
              : [3500, 8000, 16000, 28000];
          function runHardSla(attempt, ms, why) {
              try {
                if (pageUnloading()) return;
                if (global.__GR_SKIP_IDENTITY__) return;
                // Schedule final / silicon cool: never reopen probe.
                if (global.__GR_STOP_PROBE__ && global.__GR_SKIP_IDENTITY__) return;
                if (global.__GR_BRAIN_SCHEDULE_FINAL__ || global.__GR_HARD_FINAL__) return;
                var rec = global.__GR_RECEIVED_BATCHES__ || [];
                var hasB10 = false;
                for (var i = 0; i < rec.length; i++) {
                  if (String(rec[i].batch_id || "") === "B10_hw_curves") {
                    hasB10 = true;
                    break;
                  }
                }
                // Trust queue sentKeys / alreadySent when RECEIVED list lags server ack.
                try {
                  if (!hasB10 && global.GRUploadQueue) {
                    var item = {
                      session_id: sessionId || cycleHint,
                      batch_id: "B10_hw_curves",
                      source: "main",
                    };
                    if (
                      typeof global.GRUploadQueue.alreadySent === "function" &&
                      global.GRUploadQueue.alreadySent(item)
                    ) {
                      hasB10 = true;
                    }
                  }
                } catch (eSk) {}
                // B10 still pending in upload queue — wait, do not re-kick thrash.
                try {
                  if (!hasB10 && global.GRUploadQueue) {
                    // iss/70 P0: exact batch material state (queued/uploading/retry_wait/acked).
                    if (typeof GRUploadQueue.hasPendingCapture === "function") {
                      if (
                        GRUploadQueue.hasPendingCapture(
                          "B10_hw_curves",
                          sessionId || cycleHint
                        )
                      ) {
                        return;
                      }
                    }
                    if (typeof GRUploadQueue.materialState === "function") {
                      var mSt = GRUploadQueue.materialState(
                        "B10_hw_curves",
                        sessionId || cycleHint
                      );
                      if (
                        mSt === "queued" ||
                        mSt === "uploading" ||
                        mSt === "retry_wait" ||
                        mSt === "acked"
                      ) {
                        return;
                      }
                    }
                    if (GRUploadQueue.stats) {
                      var stQ = GRUploadQueue.stats() || {};
                      if ((stQ.hard_anchor_pending || 0) > 0) return;
                    }
                  }
                } catch (eK) {}
                // Already collected (kicked) but upload still settling — do not re-collect.
                try {
                  if (
                    !hasB10 &&
                    global.GRPackLoader &&
                    GRPackLoader.alreadyKicked
                  ) {
                    var ak = GRPackLoader.alreadyKicked() || {};
                    if (ak["B10_hw_curves"] && global.GRUploadQueue) {
                      // Collect done; transport owns the rest (scheduleRetry path).
                      if (
                        typeof GRUploadQueue.hasPendingCapture === "function" &&
                        GRUploadQueue.hasPendingCapture(
                          "B10_hw_curves",
                          sessionId || cycleHint
                        )
                      ) {
                        return;
                      }
                      // kicked + no pending: may be mid-enqueue race — still avoid clearKicked
                      // unless attempts exhausted and no capture (true collect miss).
                      if (ak["B10_hw_curves"]) {
                        // Prefer transport retry over GPU re-run when capture exists in queue.
                        // If no pending capture, fall through only when not recently kicked.
                      }
                    }
                  }
                } catch (eAk) {}
                // Server last analyze already has silicon — do not thrash SLA re-kick.
                try {
                  if (!hasB10 && global.__GR_LAST_ANALYZE__) {
                    var la = global.__GR_LAST_ANALYZE__;
                    var res = la.result || la;
                    var cov = (res && res.coverage) || {};
                    var cps = (res && res.cycle_probe_status) || {};
                    if (
                      cov.has_b10 === true ||
                      cps.has_b10 === true ||
                      res.b10_present === true ||
                      (res.device && res.device.hard_materials_present)
                    ) {
                      hasB10 = true;
                    }
                  }
                } catch (eLa) {}
                if (hasB10) return;
                // iss/70 + self-heal: if kicked AND transport owns capture, skip re-collect.
                // If kicked but no pending/ack (orphan collect), allow SLA re-collect — was a dead hole.
                try {
                  if (
                    global.GRPackLoader &&
                    GRPackLoader.alreadyKicked
                  ) {
                    var ak2 = GRPackLoader.alreadyKicked() || {};
                    if (ak2["B10_hw_curves"]) {
                      var ownTransport = false;
                      try {
                        if (
                          global.GRUploadQueue &&
                          GRUploadQueue.hasPendingCapture &&
                          GRUploadQueue.hasPendingCapture(
                            "B10_hw_curves",
                            sessionId || cycleHint
                          )
                        ) {
                          ownTransport = true;
                        }
                        if (
                          !ownTransport &&
                          GRUploadQueue.materialState
                        ) {
                          var stAk = GRUploadQueue.materialState(
                            "B10_hw_curves",
                            sessionId || cycleHint
                          );
                          if (
                            stAk === "queued" ||
                            stAk === "uploading" ||
                            stAk === "retry_wait" ||
                            stAk === "acked"
                          ) {
                            ownTransport = true;
                          }
                        }
                      } catch (eOwn) {}
                      if (ownTransport) {
                        // Transport path owns retries.
                        return;
                      }
                      // Orphan kicked → fall through to clearKicked + re-collect.
                    }
                  }
                } catch (eAk2) {}
                // Clear false STOP from thin analyze so hard can run.
                if (global.__GR_STOP_PROBE__ && !global.__GR_SKIP_IDENTITY__) {
                  global.__GR_STOP_PROBE__ = false;
                  global.__GR_HALT_UPLOADS__ = false;
                  try {
                    if (Q && Q.resume) Q.resume(sessionId || cycleHint);
                  } catch (eR) {}
                }
                global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
                global.__GR_KICK_MS__.hard_sla_retry = attempt + 1;
                global.__GR_KICK_MS__.hard_sla_why = why || "timer";
                // Ops: only first + last attempt (hard_sla_retry was top warn flood).
                try {
                  if (
                    global.GROps &&
                    GROps.report &&
                    (attempt === 0 || attempt === delays.length - 1)
                  ) {
                    GROps.report(
                      "hard_sla_retry",
                      "kick",
                      {
                        attempt: attempt + 1,
                        ms: ms,
                        why: why || "timer",
                        max: delays.length,
                        recollect: true,
                      },
                      attempt === delays.length - 1 ? "warn" : "info"
                    );
                  }
                } catch (eOp) {}
                // True collect miss only: clear kicked + allow re-run.
                // (Transport fail no longer clears kicked — iss/70 P0.)
                try {
                  if (global.GRPackLoader && GRPackLoader.clearKicked) {
                    GRPackLoader.clearKicked("B10_hw_curves");
                    GRPackLoader.clearKicked("B7_sandbox");
                  }
                  if (global.GRUploadQueue && GRUploadQueue.clearSentKeys) {
                    GRUploadQueue.clearSentKeys([
                      {
                        session_id: sessionId || cycleHint,
                        batch_id: "B10_hw_curves",
                        source: "main",
                      },
                    ]);
                  }
                } catch (eClr) {}
                // Content-gate before hard SLA B10 re-kick (avoid v1 sticky uploads).
                var svSla =
                  scriptVersion() ||
                  global.__GR_SERVER_PRODUCT_VERSION__ ||
                  global.__GR_PRODUCT_VERSION__ ||
                  "";
                Promise.resolve(ensurePacksMatchServerVersion(String(svSla || "")))
                  .catch(function () {
                    return false;
                  })
                  .then(function (packsOk) {
                  if (!hardPacksReady() || !packsOk) {
                    // iss/62 P1 (b10_sla_miss): one-time hard module reload fallback.
                    // ensurePacksMatchServerVersion only reloads on content mismatch;
                    // when versions match but the registry was wiped by a swap race,
                    // force a clean hard re-register once so later attempts land.
                    try {
                      if (!global.__GR_HARD_SLA_RELOAD_DONE__) {
                        global.__GR_HARD_SLA_RELOAD_DONE__ = 1;
                        resetCollectorModuleState();
                        ensureStaticHardModules();
                      }
                    } catch (eRl) {}
                    try {
                      if (global.GROps) {
                        GROps.wave2Empty({
                          hard_sla: true,
                          attempt: attempt + 1,
                          why: why || "timer",
                          packs_ok: !!packsOk,
                          algo: liteAlgoId(),
                        });
                      }
                    } catch (eW) {}
                    return;
                  }
                  var Col = global.GRCollectors;
                  var def = Col && Col.get && Col.get("B10_hw_curves");
                  if (!def || typeof def.run !== "function") return;
                  var packs = [
                    {
                      id: "B10_hw_curves",
                      pack_id: "B10_hw_curves",
                      batch_id: "B10_hw_curves",
                      priority: 1000,
                      schedule: "static",
                      run: function (c) {
                        return def.run(c);
                      },
                    },
                  ];
                  var b7 = Col.get && Col.get("B7_sandbox");
                  if (b7 && typeof b7.run === "function") {
                    packs.push({
                      id: "B7_sandbox",
                      pack_id: "B7_sandbox",
                      batch_id: "B7_sandbox",
                      priority: 900,
                      schedule: "static",
                      run: function (c) {
                        return b7.run(c);
                      },
                    });
                  }
                  // Allow re-kick even if markKicked earlier
                  try {
                    if (global.GRPackLoader && GRPackLoader.clearKicked) {
                      GRPackLoader.clearKicked("B10_hw_curves");
                      GRPackLoader.clearKicked("B7_sandbox");
                    }
                  } catch (eCk) {}
                  var c2 = {
                    session_id: sessionId || cycleHint,
                    queue: Q,
                    apiBase: apiBase,
                    site_id: siteId,
                  };
                  if (global.GRPackLoader && GRPackLoader.kickAll) {
                    return GRPackLoader.kickAll(packs, c2);
                  }
                });
              } catch (eSla) {}
          }
          delays.forEach(function (ms, attempt) {
            setTimeout(function () {
              runHardSla(attempt, ms, "timer");
            }, ms);
          });
          // Exposed for visibility=visible resume (bindHide onVisible).
          global.__GR_REQUEST_HARD_SLA__ = function (why) {
            try {
              runHardSla(delays.length, 0, why || "manual");
            } catch (eReq) {}
            try {
              if (global.GRProbeSelfHeal && GRProbeSelfHeal.onVisible) {
                GRProbeSelfHeal.onVisible();
              }
            } catch (eSh2) {}
          };
          // Start Gap Table self-heal supervisor (T/C/B) for page lifetime.
          try {
            if (global.GRProbeSelfHeal && GRProbeSelfHeal.start) {
              GRProbeSelfHeal.start({
                need_hard_anchor: true,
                missing: ["B10_hw_curves", "B0_bootstrap"],
              });
            }
          } catch (eStartHeal) {}
        })();

        var kickMid = kick1
          .then(function () {
            // Wait for the already-started L1 rest before warming full
            // modules; hard/B10 scheduling is independent through kick2.
            return restP || Promise.resolve([]);
          })
          .then(function () {
            return fullModP || ensureFullModules();
          })
          .then(function () {
            return sleep(30);
          })
          .then(function () {
            if (pageUnloading() || global.__GR_STOP_PROBE__ || opts.eagerMid === false) {
              return { kicked: [] };
            }
            return eagerMidAndB8(ctx);
          });

        // After first hard upload: one analyze nudge + B8 rejoin (server also debounces rest packs).
        try {
          var onUp = function (ev) {
            var bid = ev && ev.detail && ev.detail.batch_id;
            if (
              bid === "B0_bootstrap" ||
              bid === "B1_conflict" ||
              bid === "B2_hardware" ||
              bid === "B3_system" ||
              bid === "B12_anti_camouflage"
            ) {
              if (!global.__GR_KICK_MS__.first_hard_upload_ms) {
                global.__GR_KICK_MS__.first_hard_upload_ms = Date.now() - tBoot;
              }
              if (!global.__GR_ANALYZE_NUDGED__) {
                global.__GR_ANALYZE_NUDGED__ = true;
                postJson(
                  apiBase + "/v1/session/" + encodeURIComponent(sessionId) + "/analyze",
                  {},
                  { priority: "high" }
                ).catch(function () {});
              }
              if (bid === "B0_bootstrap") {
                fireEarlyB8(
                  apiBase,
                  global.__GR_GW_BASE__ || gwBase,
                  sessionId,
                  siteId,
                  injectPath,
                  global.__GR_VTID__ || vt
                ).catch(function () {});
                try {
                  global.removeEventListener("gr-upload-ok", onUp);
                } catch (eR) {}
              }
            }
          };
          global.addEventListener("gr-upload-ok", onUp);
          setTimeout(function () {
            try {
              global.removeEventListener("gr-upload-ok", onUp);
            } catch (eR2) {}
          }, 8000);
        } catch (eUp) {}

        // Apply open result when it arrives (cool / seed / challenge / biz) without blocking kick.
        var openApplied = openP.then(function (opened) {
          opened = opened || {};
          try {
            applyOpenEntitlement(opened);
          } catch (eEnt) {}
          var session = opened.session || {};
          var sid =
            (session.cycle_id ||
              opened.cycle_id ||
              session.session_id ||
              cycleHint ||
              "") + "";
          // Prefer server cycle id when it differs. Client mint races B0 before open,
          // but if the sticky hint was a *completed* bag the server mints a new id —
          // keeping the client mint causes all identity ingest to 410 (B8-only symptom).
          if (sid && sid !== cycleHint) {
            var deadHint = cycleHint;
            global.__GR_KICK_MS__.sid_mismatch = {
              client: deadHint,
              server: sid,
              adopted: true,
              phase: opened.phase || session.phase || "",
            };
            sessionId = sid;
            cycleHint = sid;
            ctx.session_id = sid;
            try {
              Q.configure({ session_id: sid });
            } catch (eCfg) {}
            global.__GR_SESSION_ID__ = sid;
            global.__GR_CYCLE_ID__ = sid;
            global.__GR_CYCLE_HINT__ = sid;
            try {
              if (global.GRStorage && GRStorage.setCycleId)
                GRStorage.setCycleId(
                  sid,
                  productVerBoot ||
                    (global.__GR_BOOT__ && global.__GR_BOOT__.version) ||
                    undefined
                );
              else {
                writeStickyCookie(cycleCookieName(), sid, 86400);
                expireCookieEverywhere("gr_cycle_v1");
                expireCookieEverywhere("_g5_c");
              }
            } catch (eCk) {}
            // Re-tag any pending queue items still holding the dead client mint.
            try {
              if (Q.rewriteSessionId) Q.rewriteSessionId(deadHint, sid);
            } catch (eRw) {
              /* optional */
            }
            // Seal grant is for server sid — wake grant-waiting B0/B1 uploads.
            try {
              if (Q.kick) Q.kick();
            } catch (eK0) {}
            // Re-kick B8 onto the live bag (orphan B8 may have landed on superseded id).
            try {
              global.__GR_EARLY_B8__ = null;
              fireEarlyB8(
                apiBase,
                global.__GR_GW_BASE__ || gwBaseEarly,
                sid,
                siteId,
                injectPath,
                global.__GR_VTID__ || vt
              ).then(function (b8) {
                if (earlyB8Landed(b8)) {
                  global.__GR_EARLY_B8__ = b8;
                  global.__GR_KICK_MS__.b8_after_supersede = true;
                }
              });
            } catch (eB8s) {}
          } else if (sid) {
            sessionId = sid;
            global.__GR_SESSION_ID__ = sid;
            global.__GR_CYCLE_ID__ = sid;
          }
          var phase = opened.phase || session.phase || "active";
          // Server force_identity_probe wins over stale local cool (version re-probe).
          var forceIdentity = !!(
            opened.force_identity_probe ||
            opened.client_hint_superseded ||
            (opened.reprobe_reason && String(opened.reprobe_reason).length)
          );
          if (forceIdentity) {
            try {
              global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
              global.__GR_KICK_MS__.force_identity = true;
              global.__GR_KICK_MS__.reprobe_reason =
                opened.reprobe_reason ||
                (opened.client_hint_superseded ? "client_hint_superseded" : "force");
              if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
            } catch (eFi) {}
          }
          // Sticky resume without B10: re-kick hard anchors (do not mint new cycle).
          // Soft reload (esp. gecko/Edge) often resumes sticky incomplete with only L1
          // lite → empty_anchor dg_ unless hard packs re-kick immediately.
          // Gap Table self-heal: seed missing / need_hard from open (cross-page resume).
          try {
            if (global.GRProbeSelfHeal && GRProbeSelfHeal.seedFromOpen) {
              GRProbeSelfHeal.seedFromOpen(opened);
            } else if (global.GRProbeSelfHeal && GRProbeSelfHeal.start) {
              GRProbeSelfHeal.start({
                need_hard_anchor: !!opened.need_hard_anchor,
                missing: opened.missing_batches || [],
              });
            }
          } catch (eHealOpen) {}
          if (opened.need_hard_anchor || opened.resumed || opened.converged_to_active) {
            try {
              global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
              global.__GR_KICK_MS__.need_hard_anchor = !!opened.need_hard_anchor;
              global.__GR_KICK_MS__.resumed = !!opened.resumed;
              global.__GR_KICK_MS__.converged_to_active = !!opened.converged_to_active;
              if (opened.need_hard_anchor && global.GROps && GROps.report) {
                GROps.report(
                  "need_hard_anchor",
                  "open",
                  { session_id: sessionId, resumed: !!opened.resumed },
                  "info"
                );
              }
              if (opened.converged_to_active && global.GROps && GROps.report) {
                GROps.report(
                  "cycle_converged",
                  "open",
                  { session_id: sessionId },
                  "info"
                );
              }
              // Immediate hard re-kick path (0/1.2/3.5s) — do not wait only for late SLA.
              if (opened.need_hard_anchor || opened.resumed) {
                try {
                  global.__GR_STOP_PROBE__ = false;
                  global.__GR_HALT_UPLOADS__ = false;
                  global.__GR_SKIP_IDENTITY__ = false;
                  if (global.GRUploadQueue && GRUploadQueue.resume) {
                    GRUploadQueue.resume(sessionId);
                  }
                } catch (eClr) {}
                // Short-visit hard re-kick: earlier first retries (0/0.6/1.8/4/8s).
                [0, 600, 1800, 4000, 8000].forEach(function (ms) {
                  setTimeout(function () {
                    try {
                      if (pageUnloading()) return;
                      if (global.__GR_SKIP_IDENTITY__) return;
                      var rec = global.__GR_RECEIVED_BATCHES__ || [];
                      for (var i = 0; i < rec.length; i++) {
                        if (String(rec[i].batch_id || "") === "B10_hw_curves") return;
                      }
                      ensureStaticHardModules().then(function () {
                        if (!hardPacksReady()) {
                          try {
                            if (global.GROps) {
                              GROps.wave2Empty({ soft_reload_hard: true, ms: ms });
                            }
                          } catch (eW) {}
                          // Last-chance reload of hard pack script tag when module still missing.
                          if (ms >= 3500) {
                            try {
                              if (global.ensureModulesForRoutePlan) {
                                global.ensureModulesForRoutePlan({
                                  packs: [{ pack_id: "B10_hw_curves" }, { pack_id: "B7_sandbox" }],
                                });
                              }
                            } catch (eR) {}
                          }
                          return;
                        }
                        var Col = global.GRCollectors;
                        var def = Col && Col.get && Col.get("B10_hw_curves");
                        if (!def || typeof def.run !== "function") return;
                        try {
                          if (global.GRPackLoader && GRPackLoader.clearKicked) {
                            GRPackLoader.clearKicked("B10_hw_curves");
                            GRPackLoader.clearKicked("B7_sandbox");
                          }
                        } catch (eCk) {}
                        var packs = [
                          {
                            id: "B10_hw_curves",
                            pack_id: "B10_hw_curves",
                            batch_id: "B10_hw_curves",
                            priority: 1000,
                            schedule: "static",
                            run: function (c) {
                              return def.run(c);
                            },
                          },
                        ];
                        var b7 = Col.get && Col.get("B7_sandbox");
                        if (b7 && typeof b7.run === "function") {
                          packs.push({
                            id: "B7_sandbox",
                            pack_id: "B7_sandbox",
                            batch_id: "B7_sandbox",
                            priority: 900,
                            schedule: "static",
                            run: function (c) {
                              return b7.run(c);
                            },
                          });
                        }
                        var c2 = {
                          session_id: sessionId,
                          queue: global.GRUploadQueue,
                          apiBase: apiBase,
                          site_id: siteId,
                        };
                        if (global.GRPackLoader && GRPackLoader.kickAll) {
                          return GRPackLoader.kickAll(packs, c2);
                        }
                      });
                    } catch (eSoftHard) {}
                  }, ms);
                });
              }
            } catch (eNh) {}
          }
          // Adopt server visitor_terminal_id (single VT contract).
          try {
            var ovt = opened.visitor_terminal_id || (opened.session && opened.session.visitor_terminal_id);
            if (ovt && global.GRStorage && GRStorage.adoptVt) {
              var av = GRStorage.adoptVt(ovt);
              global.__GR_VTID__ = av;
            } else if (ovt) {
              global.__GR_VTID__ = String(ovt);
            }
          } catch (eVt) {}
          // Write back server cycle id (convergence / supersede adopt).
          try {
            if (sessionId && global.GRStorage && GRStorage.setCycleId) {
              GRStorage.setCycleId(
                sessionId,
                opened.product_version || productVerBoot || global.__GR_PRODUCT_VERSION__
              );
            }
            global.__GR_SESSION_ID__ = sessionId;
            global.__GR_CYCLE_ID__ = sessionId;
            global.__GR_CYCLE_HINT__ = sessionId;
          } catch (eCy) {}
          // Server product_version self-heal (inject lag).
          try {
            var srvPv = opened.product_version || (opened.policy && opened.policy.product_version);
            if (srvPv) maybeSelfHealVersion(String(srvPv));
          } catch (eSh) {}
          // Cool/skip only when server says cool AND last identity has silicon (B10/residual).
          // Thin cool was the production failure mode requiring users to clear cache.
          function lastHasSilicon(res) {
            try {
              if (!res || typeof res !== "object") return false;
              var d = res.device || (res.result && res.result.device) || {};
              var dig = String(d.digest_path || res.digest_path || "");
              var tier = String(d.device_tier || res.device_tier || "");
              var f = res.fields || (res.result && res.result.fields) || {};
              var rs = f.residual_std != null || f.residual_mean != null || f.residual_ok === true;
              var hw = f.hw_curve_webgl != null || f.hw_curve_audio != null;
              return (rs || hw) && (dig.indexOf("real_curves") >= 0 || tier === "dh" || tier === "dv");
            } catch (eS) {
              return false;
            }
          }
          var lastId =
            opened.last_identity_result ||
            (session && session.last_identity_result) ||
            null;
          var cpsOpen = opened.cycle_probe_status || {};
          // Cool skip only when brain schedule final (coverage complete / analysis terminal).
          // commercial_identity_final / dh_ alone is NOT enough (maximize probe v5.8.53+).
          var scheduleFinal =
            cpsOpen.brain_schedule_final === true ||
            cpsOpen.final_analysis_ok === true ||
            cpsOpen.cycle_status === "complete" ||
            opened.cycle_status === "complete" ||
            (cpsOpen.coverage_complete === true &&
              (cpsOpen.probe_complete === true || cpsOpen.analysis_terminal === true));
          var serverWantsCool =
            phase === "cool" ||
            opened.business_state === "identity_complete_cool" ||
            opened.skip_identity_probe ||
            opened.skip_session_probe ||
            (session && session.skip_identity_probe);
          var siliconCoolOk = lastHasSilicon(lastId) || opened.cool_silicon_ok === true;
          if (serverWantsCool && !siliconCoolOk && !forceIdentity) {
            // Treat as force re-probe without user cache clear.
            forceIdentity = true;
            try {
              global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
              global.__GR_KICK_MS__.cool_blocked_no_silicon = true;
              if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
              if (global.GROps && GROps.report) {
                // Expected self-heal path (force re-probe) — info, not operational failure.
                // Prod v150: Ubuntu Firefox cool-then-silicon still fully probed.
                GROps.report(
                  "cool_without_silicon",
                  "open",
                  {
                    phase: phase,
                    action: "force_reprobe",
                    note: "server cool without last silicon; FE forces re-probe",
                  },
                  "info"
                );
              }
            } catch (eCb) {}
          }
          // Prior cool stamped before schedule final (v5.8.51 bug path): re-probe to finish soft.
          if (serverWantsCool && !scheduleFinal && !forceIdentity) {
            forceIdentity = true;
            try {
              global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
              global.__GR_KICK_MS__.cool_blocked_schedule_incomplete = true;
              if (global.GRStorage && GRStorage.setCoolUntil) GRStorage.setCoolUntil(0);
              if (global.GROps && GROps.report) {
                GROps.report(
                  "cool_without_schedule_final",
                  "open",
                  { phase: phase, commercial: !!cpsOpen.commercial_identity_final },
                  "warn"
                );
              }
            } catch (eCs) {}
          }
          var skipIdentity =
            !forceIdentity && !!serverWantsCool && siliconCoolOk && scheduleFinal;
          try {
            if (opened.cycle_probe_status) {
              global.__GR_CYCLE_PROBE_STATUS__ = opened.cycle_probe_status;
            }
            if (opened.business_state) {
              global.__GR_BUSINESS_STATE__ = opened.business_state;
            }
          } catch (eSt) {}
          // Biz board taxonomy: browser|robots (server also normalizes legacy js/nojs).
          var facet = opened.visitor_facet || "browser";
          if (facet === "js" || facet === "nojs") facet = "browser";
          var openedVt =
            opened.visitor_terminal_id ||
            session.visitor_terminal_id ||
            opts.visitor_terminal_id ||
            vt ||
            "";
          global.__GR_VISITOR_FACET__ = facet;
          if (openedVt) global.__GR_VTID__ = openedVt;
          reportBizVisit(apiBase, {
            site_id: siteId || opened.site_id || "",
            visitor_terminal_id: openedVt,
            visitor_facet: facet,
            session_id: sessionId,
            page_host:
              typeof location !== "undefined" ? location.hostname || "" : "",
            event: "open_ok",
            events: ["sdk_load", "open_ok", "facet"],
            summary: { source: "gr.boot", phase: phase },
          });
          if (opened.challenge_seed) {
            global.__GR_CHALLENGE_SEED__ = opened.challenge_seed;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.challenge_seed = opened.challenge_seed;
            global.__GR_BOOT__.challenge_seed_ttl_ms =
              opened.challenge_seed_ttl_ms ||
              (opened.policy && opened.policy.challenge_seed_ttl_ms) ||
              120000;
            global.__GR_BOOT__.challenge_seed_exp_ms = opened.challenge_seed_exp_ms || null;
            global.__GR_BOOT__.challenge_seed_sig = opened.challenge_seed_sig || null;
            global.__GR_CHALLENGE_SEED_SIG__ = opened.challenge_seed_sig || null;
            global.__GR_CHALLENGE_SEED_EXP__ = opened.challenge_seed_exp_ms || null;
          }
          // Session seal grant (ephemeral key → /v1/ingest/sealed; master secret never in FE)
          // CRITICAL: always stash on durable globals FIRST — race/entry re-eval wipes closed-over grant.
          try {
            var needSeal =
              opened.require_sealed_ingest === true ||
              (opened.policy && opened.policy.require_sealed_ingest === true) ||
              !!global.__GR_REQUIRE_SEALED__ ||
              !!global.__GR_SEEN_SEALED_REQUIRED__;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            if (needSeal) {
              global.__GR_REQUIRE_SEALED__ = true;
              global.__GR_BOOT__.require_sealed_ingest = true;
            }
            if (opened.seal_grant) {
              global.__GR_SEAL_GRANT__ = opened.seal_grant;
              global.__GR_BOOT__.seal_grant = opened.seal_grant;
            }
            if (global.GRSeal) {
              if (needSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
              if (opened.seal_grant && GRSeal.setGrant) GRSeal.setGrant(opened.seal_grant);
              if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            }
            // Wake upload queue: B0 may be waiting on seal_grant (race_no_open_gate).
            try {
              if (global.GRUploadQueue && typeof GRUploadQueue.kick === "function") {
                GRUploadQueue.kick();
              } else if (global.GRUploadQueue && typeof GRUploadQueue.pump === "function") {
                GRUploadQueue.pump();
              }
            } catch (eKick) {}
          } catch (eSealG) {}
          global.__GR_PHASE__ = phase;
          global.__GR_SKIP_IDENTITY__ = skipIdentity;
          // Adopt server cycle id when sticky hint was superseded (no user cache clear).
          if (sessionId && global.GRStorage && GRStorage.setCycleId) {
            GRStorage.setCycleId(
              sessionId,
              opened.product_version || productVerBoot || global.__GR_PRODUCT_VERSION__
            );
          }
          try {
            if (global.GRProbeLifecycle && GRProbeLifecycle.deriveFromOpen) {
              GRProbeLifecycle.deriveFromOpen(opened);
            }
          } catch (eDer) {}
          // Only persist cool when silicon-validated skip (never invent 24h cool for thin).
          if (skipIdentity && siliconCoolOk && global.GRStorage && GRStorage.setCoolUntil) {
            var cu =
              opened.cool_until_ms ||
              session.cool_until_ms ||
              Date.now() + 24 * 60 * 60 * 1000;
            if (cu > Date.now()) {
              GRStorage.setCoolUntil(
                cu,
                opened.product_version || productVerBoot || global.__GR_PRODUCT_VERSION__
              );
            }
            global.__GR_PHASE__ = "cool";
          } else if (!skipIdentity && global.GRStorage && GRStorage.setCoolUntil) {
            // Incomplete gap-fill: do not clear cool if server still has cool (edge).
            if (!(opened.cool_until_ms > Date.now())) {
              GRStorage.setCoolUntil(0);
            }
          }
          // Cool / skip: stop queue + further pack kicks immediately (race may have enqueued).
          if (skipIdentity) {
            try {
              if (Q && Q.halt) Q.halt("cool", sessionId, "cool");
              else if (Q && Q.flush) {
                /* no-op */
              }
            } catch (eHalt) {}
            global.__GR_STOP_PROBE__ = true;
            global.__GR_HALT_UPLOADS__ = true;
            global.__GR_SKIP_IDENTITY__ = true;
          } else {
            // Fresh active cycle: clear previous halt for other session ids.
            try {
              if (Q && Q.resume) Q.resume(sessionId);
            } catch (eRes) {}
          }
          var received = opened.received_batches || [];
          if (Q.seedSentKeys) {
            Q.seedSentKeys(
              received.map(function (b) {
                return {
                  session_id: sessionId,
                  batch_id: b.batch_id,
                  source: b.source || "main",
                };
              })
            );
          }
          if (global.GRPackLoader && GRPackLoader.markKicked) {
            received.forEach(function (b) {
              if (b.batch_id) GRPackLoader.markKicked(b.batch_id);
              if (b.batch_id === "B8_gateway") GRPackLoader.markKicked("B8_gateway_early");
              if (b.batch_id === "B0_bootstrap") GRPackLoader.markKicked("B0_bootstrap");
            });
          }
          global.__GR_RECEIVED_BATCHES__ = received;
          global.__GR_KICK_MS__.skipped_received = received.length;
          if (skipIdentity) {
            if (opened.last_identity_result) {
              global.__GR_LAST_ANALYZE__ = {
                result: opened.last_identity_result,
                route_plan:
                  opened.last_identity_result.route_plan ||
                  (opened.last_identity_result.brain &&
                    opened.last_identity_result.brain.route_plan) ||
                  null,
              };
            }
            global.__GR_STOP_PROBE__ = true;
            return {
              cool: true,
              session_id: sessionId,
              cycle_id: sessionId,
              phase: phase,
              skip_identity_probe: true,
              inject_path: injectPath,
              visitor_terminal_id: session.visitor_terminal_id || vt,
              cool_until_ms: opened.cool_until_ms || session.cool_until_ms,
              last_identity_result: opened.last_identity_result || null,
              pack_health: global.GRPackLoader ? GRPackLoader.health() : null,
              multi_tick: null,
              route_plan: global.__GR_ROUTE_PLAN__ || null,
            };
          }
          return { cool: false, opened: opened, session: session };
        });

        function afterWave1(openInfo) {
          if (openInfo && openInfo.cool) {
            if (typeof opts.onDone === "function") opts.onDone(openInfo);
            return openInfo;
          }
          global.__GR_KICK_MS__.wave1_settled = Date.now();
          var packsAll = wave1.concat(wave2);
          var session = (openInfo && openInfo.session) || {};
          var out = {
            session_id: sessionId,
            inject_path: injectPath,
            visitor_terminal_id: session.visitor_terminal_id || vt,
            pack_health: global.GRPackLoader && GRPackLoader.health ? GRPackLoader.health() : null,
            pack_kick_order: global.__GR_PACK_KICK_ORDER__,
            collectors_registered:
              global.GRCollectors && GRCollectors.count ? GRCollectors.count() : 0,
            l1: true,
            static_packs: packsAll,
            static_wave1: wave1.slice(),
            static_wave2: wave2.slice(),
            queue: Q.stats(),
            sandbox: global.__GR_SANDBOX_RESULT__ || null,
            multi_tick: null,
            route_plan: null,
            kick_ms: global.__GR_KICK_MS__,
          };

          var enableMulti = opts.multiTick !== false;
          var maxTicks =
            opts.maxTicks != null
              ? opts.maxTicks
              : (global.GRProbeLifecycle &&
                  global.GRProbeLifecycle.POLICY &&
                  global.GRProbeLifecycle.POLICY.multi_tick_max) ||
                16;
          // Short settle: first analyze ASAP while wave2 still uploading (maximize speed).
          var idleMs = opts.settleMs != null ? opts.settleMs : 150;

          function finish(multi) {
            out.multi_tick = multi;
            out.route_plan = global.__GR_ROUTE_PLAN__ || null;
            out.real_band =
              global.__GR_LAST_ANALYZE__ &&
              global.__GR_LAST_ANALYZE__.result &&
              global.__GR_LAST_ANALYZE__.result.real_band;
            if (typeof opts.onDone === "function") opts.onDone(out);
            return out;
          }

          if (!enableMulti) {
            var an = waitUploadIdle(idleMs)
              .then(function () {
                return fetchLatestAnalysis(apiBase, sessionId);
              })
              .then(function (result) {
                if (result) return { result: result, route_plan: result.route_plan };
                return postJson(
                  apiBase + "/v1/session/" + encodeURIComponent(sessionId) + "/analyze",
                  {}
                );
              })
              .then(function (aj) {
                out.real_band = aj.result && aj.result.real_band;
                out.route_plan = aj.route_plan || (aj.result && aj.result.route_plan);
                global.__GR_ROUTE_PLAN__ = out.route_plan;
                if (typeof opts.onDone === "function") opts.onDone(out);
                return out;
              });
            if (opts.waitAnalyze === false) {
              an.catch(function () {});
              if (typeof opts.onDone === "function") opts.onDone(out);
              return out;
            }
            return an;
          }

          var loopP = multiTickLoop(ctx, maxTicks, idleMs).then(function (multi) {
            out.multi_tick = multi;
            // Multi-party: always reconcile FE local vs BE cycle after multi-tick.
            return reconcileProbeStatus(apiBase, sessionId)
              .then(function (rec) {
                out.probe_reconcile = rec;
                if (global.__GR_STOP_PROBE__ || pageUnloading() || global.__GR_CYCLE_CLOSED__) {
                  return finish(multi);
                }
                // Long follow-up so mid/dense B packs continue after identity-core (maximize).
                return coverageFollowup(
                  ctx,
                  opts.coverageFollowMs != null ? opts.coverageFollowMs : 45000
                ).then(function (fu) {
                  out.coverage_followup = fu;
                  // Second reconcile after followup (catch late complete/410).
                  return reconcileProbeStatus(apiBase, sessionId).then(function (rec2) {
                    out.probe_reconcile_followup = rec2;
                    return finish(multi);
                  });
                });
              })
              .catch(function () {
                return finish(multi);
              });
          });
          kick2.catch(function () {});
          kickMid
            .then(function (r) {
              out.eager_mid = r;
              global.__GR_KICK_MS__.eager_mid_kicked = (r && r.kicked) || [];
            })
            .catch(function () {});
          if (opts.waitAnalyze === false) {
            loopP.catch(function () {});
            if (typeof opts.onDone === "function") {
              loopP.then(function (full) {
                opts.onDone(full);
              });
            }
            return out;
          }
          return loopP;
        }

        // Start multi-tick after wave1 kick settles; open cool may short-circuit.
        return Promise.all([kick1.catch(function () { return []; }), openApplied])
          .then(function (pair) {
            return afterWave1(pair[1]);
          })
          .catch(function () {
            return afterWave1({ cool: false, session: {} });
          });
      });
      global.__GR_PIPELINE_P__ = pipelineP;
      return pipelineP;
    },
  };

  global.GRBoot = Boot;

  /**
   * Business link fields (site-controlled). Merged into every ingest payload.fields.
   * Never put plaintext email/phone/user_id — use HMAC subject_ref or opaque tokens.
   * Example: GR.setLinkFields({ custom_link: { id: "sub_v1_...", cohort: "seller" } })
   */
  var __linkFields = global.__GR_LINK_FIELDS__ || {};
  function setLinkFields(obj) {
    if (!obj || typeof obj !== "object") return __linkFields;
    __linkFields = __linkFields || {};
    try {
      if (obj.custom_link != null) __linkFields.custom_link = obj.custom_link;
      if (obj.client_tags != null) __linkFields.client_tags = obj.client_tags;
      // flat pass-through of other non-PII keys (server sanitizes)
      for (var k in obj) {
        if (!Object.prototype.hasOwnProperty.call(obj, k)) continue;
        if (k === "custom_link" || k === "client_tags") continue;
        if (/email|phone|password|user_id|userid|ssn|card/i.test(k)) continue;
        __linkFields[k] = obj[k];
      }
    } catch (e) {}
    global.__GR_LINK_FIELDS__ = __linkFields;
    return __linkFields;
  }
  function getLinkFields() {
    return __linkFields || global.__GR_LINK_FIELDS__ || {};
  }
  /** Merge link fields into a fields object (for upload_queue / collectors). */
  function applyLinkFields(fields) {
    var f = fields && typeof fields === "object" ? fields : {};
    var lf = getLinkFields();
    try {
      if (lf.custom_link != null) f.custom_link = lf.custom_link;
      if (lf.client_tags != null) f.client_tags = lf.client_tags;
      for (var k in lf) {
        if (!Object.prototype.hasOwnProperty.call(lf, k)) continue;
        if (k === "custom_link" || k === "client_tags") continue;
        if (f[k] == null) f[k] = lf[k];
      }
    } catch (e) {}
    return f;
  }

  // Public site-facing API (CDN-safe)
  global.GR = global.GR || {};
  global.GR.setLinkFields = setLinkFields;
  global.GR.getLinkFields = getLinkFields;
  global.GR.applyLinkFields = applyLinkFields;
  global.GR.version = global.GR.version || (global.__GR_BOOT__ && global.__GR_BOOT__.version) || "";
  global.GR.Boot = Boot;

  // Hook upload queue if present: stamp link fields on every batch
  try {
    var Q = global.GRUploadQueue;
    if (Q && typeof Q.enqueue === "function" && !Q.__gr_link_hooked) {
      var _enq = Q.enqueue.bind(Q);
      Q.enqueue = function (item) {
        try {
          if (item && item.payload) {
            var pl = item.payload;
            if (!pl.fields) pl.fields = {};
            pl.fields = applyLinkFields(pl.fields);
          } else if (item && item.fields) {
            item.fields = applyLinkFields(item.fields);
          }
        } catch (eH) {}
        return _enq(item);
      };
      Q.__gr_link_hooked = true;
    }
  } catch (eQ) {}

  var auto = !(BOOT_SCRIPT && BOOT_SCRIPT.getAttribute && BOOT_SCRIPT.getAttribute("data-autostart") === "0");
  if (auto && typeof fetch === "function") {
    var policyMax =
      (global.GRProbeLifecycle &&
        global.GRProbeLifecycle.POLICY &&
        global.GRProbeLifecycle.POLICY.multi_tick_max) ||
      48;
    Boot.start({
      waitAnalyze: false,
      waitPacks: false,
      multiTick: true,
      // Full brain schedule needs enough multi-ticks (was hard-coded 8 → mid/R incomplete).
      maxTicks: policyMax,
      settleMs: 150,
      wave2DelayMs: 0,
      coverageFollowMs: 45000,
      concurrency: 3,
      ramp_after_first: 16,
      mid_ramp_concurrency: 8,
      // Eager mid/dense until schedule final — gap-fill on resume; route_plan still authority for stop.
      eagerMid: true,
    }).catch(function (err) {
      try { if (global.GROps) GROps.report("boot_err", "boot", { err: String(err && err.message || err || "") }, "error"); } catch (eOp) {}
    });
  }
})(typeof window !== "undefined" ? window : globalThis);

