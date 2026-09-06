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
