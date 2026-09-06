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
