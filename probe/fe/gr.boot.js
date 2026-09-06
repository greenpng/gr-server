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
