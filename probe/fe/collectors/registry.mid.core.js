/* green-v5 registry.mid.core — auto-split from registry.js; do not edit by hand */
(function (global) {
  "use strict";
  var C = global.GRCollectors;
  if (!C || !C.register) {
    try { console.warn("[gr] registry.mid.core: static registry not loaded"); } catch (e) {}
    return;
  }
  if (C.__midCoreLoaded || (C.get && C.get("B16_fast_signals"))) {
    C.__midCoreLoaded = true;
    return;
  }
  var register = C.register.bind(C);
  var h = C.__h || {};
  var midFields = h.midFields;
  var midEnqueue = h.midEnqueue;
  var enqueue = h.enqueue;
  var machineStableSignals = h.machineStableSignals;
  var deepMachineProbes = h.deepMachineProbes;
  var simpleHash = h.simpleHash;
  var webglResidualMean = h.webglResidualMean;
  var webglResidualCurveV3e = h.webglResidualCurveV3e;
  var webrtcHostHashQuick = h.webrtcHostHashQuick;
  var osInstanceHashQuick = h.osInstanceHashQuick;
  var detectEngineFamily = h.detectEngineFamily;
  var probeProfileForEngine = h.probeProfileForEngine;
  var deriveOsFamily = h.deriveOsFamily;
  var hwNoiseProbes = h.hwNoiseProbes;
  var stripRawSamples = h.stripRawSamples;
  var shouldUploadSamples = h.shouldUploadSamples;
  var surfaceMaterials = h.surfaceMaterials;
  var envStackFusion = h.envStackFusion;
  var rendererClassFromLabel = h.rendererClassFromLabel;
  var softwareRendererHeuristic = h.softwareRendererHeuristic;
  var webglUnitSurfaceCompact = h.webglUnitSurfaceCompact;
  var webglCapsLite = h.webglCapsLite;
  var canvasHashLite = h.canvasHashLite;
  var mathDigestLite = h.mathDigestLite;
  var fontPresenceSample = h.fontPresenceSample;
  var fieldsBootstrap = h.fieldsBootstrap;
  var identitySurfaceFields = h.identitySurfaceFields;
  var storageQuotaClass = h.storageQuotaClass;
  if (typeof midFields !== "function" || typeof enqueue !== "function") {
    try { console.warn("[gr] registry.mid.core: helpers missing from static"); } catch (e2) {}
    return;
  }

  register("B16_fast_signals", {
    priority: 70,
    schedule: "dynamic",
    batch_id: "B16_fast_signals",
    layer: "mid4",
    run: function (ctx) {
      var f = midFields("fast_signals");
      var ms = machineStableSignals();
      f.connection = ms.net_effective_type || null;
      f.downlink = ms.net_downlink;
      f.net_rtt = ms.net_rtt;
      f.net_type = ms.net_type || "";
      f.audio_sample_rate = ms.audio_sample_rate;
      f.screen_avail_width = ms.screen_avail_width;
      f.screen_avail_height = ms.screen_avail_height;
      // Performance timeline lite (nav timing)
      try {
        var nav = performance.getEntriesByType && performance.getEntriesByType("navigation");
        if (nav && nav[0]) {
          f.perf_dom_content_loaded_ms =
            nav[0].domContentLoadedEventEnd != null
              ? Math.round(nav[0].domContentLoadedEventEnd * 100) / 100
              : null;
          f.perf_load_event_ms =
            nav[0].loadEventEnd != null ? Math.round(nav[0].loadEventEnd * 100) / 100 : null;
          f.perf_ttfb_ms =
            nav[0].responseStart != null && nav[0].requestStart != null
              ? Math.round((nav[0].responseStart - nav[0].requestStart) * 100) / 100
              : null;
        }
        f.perf_now = performance.now();
        f.time_origin = performance.timeOrigin || null;
      } catch (eP) {}
      // Media prefs lite
      try {
        f.media_prefers_color_scheme = window.matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light";
        f.media_prefers_reduced_motion = !!window.matchMedia("(prefers-reduced-motion: reduce)")
          .matches;
      } catch (eM) {}
      // Merge live deep probes (WebRTC/storage/UA-CH) — real network + machine params.
      return deepMachineProbes().then(function (deep) {
        if (deep) {
          Object.keys(deep).forEach(function (k) {
            f[k] = deep[k];
          });
          if (deep.architecture) f.ua_ch_architecture = deep.architecture;
          if (deep.platform_version) f.ua_ch_platform_version = deep.platform_version;
          if (deep.storage_quota != null) f.storage_quota = deep.storage_quota;
        }
        return midEnqueue(ctx, "B16_fast_signals", f, 70);
      });
    },
  });
  register("B13_authorized", {
    priority: 68,
    schedule: "dynamic",
    batch_id: "B13_authorized",
    layer: "mid4",
    run: function (ctx) {
      return midEnqueue(
        ctx,
        "B13_authorized",
        {
          authorized_surface: true,
          cookie_enabled: !!navigator.cookieEnabled,
          local_storage: (function () {
            try {
              localStorage.setItem("__gr", "1");
              localStorage.removeItem("__gr");
              return true;
            } catch (e) {
              return false;
            }
          })(),
        },
        68
      );
    },
  });
  register("B15_cross_curves", {
    priority: 66,
    schedule: "dynamic",
    batch_id: "B15_cross_curves",
    layer: "mid4",
    run: function (ctx) {
      // Real cross-context curve compare (top vs sandbox mirror vs worker if present).
      var f = midFields("cross_curves");
      f.cross_context_algo = "gr_cross_curves_v3";
      try {
        var mainRes = null;
        try {
          mainRes = webglResidualMean();
        } catch (e0) {}
        f.main_residual_mean = mainRes && mainRes.residual_mean != null ? mainRes.residual_mean : null;
        f.main_residual_hist = mainRes && mainRes.residual_hist ? mainRes.residual_hist : null;
        var sb = global.__GR_SANDBOX_RESULT__ || {};
        f.sandbox_sources = sb.received || [];
        f.sandbox_residual_mean =
          sb.residual_mean != null
            ? sb.residual_mean
            : sb.fields && sb.fields.residual_mean != null
              ? sb.fields.residual_mean
              : null;
        // Prefer nest_residual_mean (honest nest lite) over any legacy residual_mean.
        f.nest_residual_mean =
          sb.nest_residual_mean != null
            ? sb.nest_residual_mean
            : sb.fields && sb.fields.nest_residual_mean != null
              ? sb.fields.nest_residual_mean
              : f.sandbox_residual_mean;
        f.nest_engine_family =
          sb.nest_engine_family ||
          (sb.fields && sb.fields.nest_engine_family) ||
          null;
        if (
          f.main_residual_mean != null &&
          f.sandbox_residual_mean != null &&
          typeof f.main_residual_mean === "number" &&
          typeof f.sandbox_residual_mean === "number"
        ) {
          f.cross_residual_delta = Math.abs(f.main_residual_mean - f.sandbox_residual_mean);
          f.cross_residual_match = f.cross_residual_delta < 5e-5;
        }
        var mainMean =
          f.main_residual_mean != null
            ? f.main_residual_mean
            : global.__GR_MAIN_RESIDUAL_MEAN__;
        f.nest_residual_algo =
          f.nest_residual_algo ||
          sb.nest_residual_algo ||
          (sb.fields && sb.fields.nest_residual_algo) ||
          null;
        // Prefer same-algo nest (v3b) vs commercial residual; hist-lite is not comparable.
        if (
          mainMean != null &&
          f.nest_residual_mean != null &&
          typeof mainMean === "number" &&
          typeof f.nest_residual_mean === "number"
        ) {
          var sameAlgo = /residual_hist_v3b|webgl_residual/.test(
            String(f.nest_residual_algo || "")
          );
          var sameBand =
            (mainMean >= 0.15 && f.nest_residual_mean >= 0.15) ||
            (mainMean < 0.12 && f.nest_residual_mean < 0.12);
          f.nest_vs_main_comparable = !!(sameAlgo || sameBand);
          if (f.nest_vs_main_comparable) {
            f.nest_vs_main_agree_0p001 =
              Math.abs(mainMean - f.nest_residual_mean) < 0.001;
            f.nest_vs_main_note = sameAlgo ? "same_algo_v3b" : "magnitude_band";
          } else {
            f.nest_vs_main_agree_0p001 = null;
            f.nest_vs_main_note = "algo_scale_mismatch_hist_vs_residual";
          }
        }
        // Lightweight same-page second pass for session-internal CV (anti-noise / anti-replay lite)
        try {
          var r2 = webglResidualMean();
          if (r2 && r2.residual_mean != null && f.main_residual_mean != null) {
            f.session_residual_repeat = r2.residual_mean;
            f.session_residual_cv =
              Math.abs(r2.residual_mean - f.main_residual_mean) /
              (Math.abs(f.main_residual_mean) + 1e-12);
          }
        } catch (e1) {}
        f.worker_available = typeof Worker !== "undefined";
        f.offscreen_available = typeof OffscreenCanvas !== "undefined";
        // H14 multi-field layer divergence (main vs sandbox mirror fields if present)
        try {
          var sbFields = (sb && sb.fields) || {};
          var keys = [
            "platform",
            "user_agent",
            "hardware_concurrency",
            "device_memory",
            "timezone",
            "webdriver",
            "webgl_unmasked_renderer",
          ];
          var diffs = [];
          var compared = 0;
          keys.forEach(function (k) {
            var a = f[k];
            if (a === undefined && ctx && ctx.fields) a = ctx.fields[k];
            var b = sbFields[k];
            if (a === undefined || b === undefined || b === null) return;
            compared++;
            var sa = String(a);
            var sbv = String(b);
            if (sa !== sbv) diffs.push({ key: k, main: sa.slice(0, 80), nest: sbv.slice(0, 80) });
          });
          f.layer_divergence_compared = compared;
          f.layer_divergence_n = diffs.length;
          f.layer_divergence_score =
            compared > 0 ? Math.round((1 - diffs.length / compared) * 1000) / 1000 : null;
          f.layer_divergence_sample = diffs.slice(0, 8);
          f.layer_divergence_match = compared > 0 && diffs.length === 0;
          // Scorer multi-source hedge + greepng-style named mismatches (useful→used).
          f.multi_source_match_ratio =
            compared > 0 ? Math.round((1 - diffs.length / compared) * 1000) / 1000 : null;
          f.iframe_ua_mismatch = diffs.some(function (d) { return d.key === "user_agent"; });
          f.iframe_platform_mismatch = diffs.some(function (d) { return d.key === "platform"; });
          f.sandbox_ua_mismatch = f.iframe_ua_mismatch;
          f.sandbox_platform_mismatch = f.iframe_platform_mismatch;
          f.sandbox_webdriver_mismatch = diffs.some(function (d) { return d.key === "webdriver"; });
          f.sandbox_hw_mismatch = diffs.some(function (d) {
            return d.key === "hardware_concurrency" || d.key === "device_memory";
          });
          f.sandbox_tostring_diverged = false;
          try {
            var mainTs = Function.prototype.toString.call(Function.prototype.toString);
            var nestTs = sbFields.function_tostring_sample;
            if (nestTs != null && String(nestTs) !== String(mainTs).slice(0, 120)) {
              f.sandbox_tostring_diverged = true;
            }
          } catch (eTs) {}
        } catch (eDiv) {
          f.layer_divergence_error = String((eDiv && eDiv.message) || eDiv);
        }
      } catch (eX) {
        f.cross_error = String((eX && eX.message) || eX);
      }
      f.cross_context_algo = "gr_cross_curves_v3";
      return midEnqueue(ctx, "B15_cross_curves", f, 66);
    },
  });
  register("B6_risk", {
    priority: 40,
    schedule: "dynamic",
    batch_id: "B6_risk",
    layer: "deep",
    run: function (ctx) {
      var f = {
        webdriver: !!(navigator.webdriver),
        max_touch: navigator.maxTouchPoints || 0,
        hardware_concurrency: navigator.hardwareConcurrency || null,
        plugins_length: navigator.plugins ? navigator.plugins.length : null,
        permission_notification: null,
        battery_charging: null,
        media_devices_enumerate: null,
      };
      try {
        f.automation = {
          webdriver: !!navigator.webdriver,
          playwright: !!(window._playwright || window.__playwright || navigator.webdriver),
          selenium: !!(window.document && document.$cdc_asdjflasutopfhvcZLmcfl_),
          cdc: !!(window.cdc_adoQpoasnfa76pfcZLmcfl_Array || window.cdc_adoQpoasnfa76pfcZLmcfl_Promise),
        };
      } catch (eA) {}
      var finish = function () {
        enqueue(ctx, "B6_risk", f, 40, "main");
      };
      var pending = 0;
      var done = function () {
        pending--;
        if (pending <= 0) finish();
      };
      // Readonly only — never Notification.requestPermission
      try {
        if (typeof Notification !== "undefined" && Notification.permission != null) {
          f.permission_notification = String(Notification.permission);
        }
      } catch (eP) {}
      try {
        if (navigator.getBattery) {
          pending++;
          navigator
            .getBattery()
            .then(function (b) {
              f.battery_charging = b ? !!b.charging : null;
              f.battery_level = b && b.level != null ? b.level : null;
              done();
            })
            .catch(function () {
              done();
            });
        }
      } catch (eB) {}
      try {
        if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
          pending++;
          navigator.mediaDevices
            .enumerateDevices()
            .then(function (list) {
              f.media_devices_enumerate = list ? list.length : 0;
              done();
            })
            .catch(function () {
              done();
            });
        }
      } catch (eM) {}
      if (pending === 0) finish();
    },
  });
  register("B4_mobile", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B4_mobile",
    layer: "deep",
    run: function (ctx) {
      var orient = null;
      var orientAngle = null;
      try {
        if (screen.orientation) {
          orient = screen.orientation.type || null;
          orientAngle = screen.orientation.angle != null ? screen.orientation.angle : null;
        }
      } catch (e) {}
      var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      // Display / viewport suite (no permission APIs) — desktop + mobile parity.
      var scr = typeof screen !== "undefined" ? screen : {};
      var vv = null;
      try {
        vv = window.visualViewport || null;
      } catch (eVv) {}
      function mq(q) {
        try {
          return !!(window.matchMedia && window.matchMedia(q).matches);
        } catch (eM) {
          return null;
        }
      }
      var f = {
        display_algo: "gr_display_device_v2",
        max_touch_points: navigator.maxTouchPoints != null ? navigator.maxTouchPoints : null,
        orientation: orient,
        orientation_angle: orientAngle,
        device_pixel_ratio: typeof devicePixelRatio !== "undefined" ? devicePixelRatio : null,
        // Screen metrics (all devices)
        screen_width: scr.width != null ? scr.width : null,
        screen_height: scr.height != null ? scr.height : null,
        screen_avail_width: scr.availWidth != null ? scr.availWidth : null,
        screen_avail_height: scr.availHeight != null ? scr.availHeight : null,
        screen_color_depth: scr.colorDepth != null ? scr.colorDepth : null,
        screen_pixel_depth: scr.pixelDepth != null ? scr.pixelDepth : null,
        screen_is_extended: scr.isExtended != null ? !!scr.isExtended : null,
        // Visual viewport (mobile browser chrome, pinch-zoom)
        vv_width: vv && vv.width != null ? Math.round(vv.width) : null,
        vv_height: vv && vv.height != null ? Math.round(vv.height) : null,
        vv_scale: vv && vv.scale != null ? Math.round(vv.scale * 1000) / 1000 : null,
        vv_offset_left: vv && vv.offsetLeft != null ? Math.round(vv.offsetLeft) : null,
        vv_offset_top: vv && vv.offsetTop != null ? Math.round(vv.offsetTop) : null,
        inner_width: typeof innerWidth !== "undefined" ? innerWidth : null,
        inner_height: typeof innerHeight !== "undefined" ? innerHeight : null,
        outer_width: typeof outerWidth !== "undefined" ? outerWidth : null,
        outer_height: typeof outerHeight !== "undefined" ? outerHeight : null,
        // CSS media — form/display without sensors
        mq_pointer_fine: mq("(pointer: fine)"),
        mq_pointer_coarse: mq("(pointer: coarse)"),
        mq_hover_hover: mq("(hover: hover)"),
        mq_hover_none: mq("(hover: none)"),
        mq_any_pointer_coarse: mq("(any-pointer: coarse)"),
        mq_prefers_color_scheme_dark: mq("(prefers-color-scheme: dark)"),
        mq_prefers_reduced_motion: mq("(prefers-reduced-motion: reduce)"),
        mq_dynamic_range_high: mq("(dynamic-range: high)"),
        mq_color_gamut_p3: mq("(color-gamut: p3)"),
        mq_color_gamut_srgb: mq("(color-gamut: srgb)"),
        mq_display_mode_standalone: mq("(display-mode: standalone)"),
        mq_orientation_portrait: mq("(orientation: portrait)"),
        mq_orientation_landscape: mq("(orientation: landscape)"),
        net_effective_type: conn && conn.effectiveType ? conn.effectiveType : null,
        net_rtt: conn && conn.rtt != null ? conn.rtt : null,
        net_downlink: conn && conn.downlink != null ? conn.downlink : null,
        net_save_data: conn && conn.saveData != null ? !!conn.saveData : null,
        // iss/21 U-1 / T-UA-1: UA is claim only — do NOT write form_class here
        // (B0 owns capability form_class for device_id digest).
        mobile_ua_signals: /Mobile|Android|iPhone|iPad/i.test(navigator.userAgent || ""),
        mobile_ua_claim: /Mobile|Android|iPhone|iPad/i.test(navigator.userAgent || ""),
        touch_support: "ontouchstart" in window || (navigator.maxTouchPoints || 0) > 0,
        // typeof only — do not register deviceorientation/devicemotion listeners (FF deprecation).
        sensors_motion: typeof DeviceMotionEvent !== "undefined",
        sensors_orient: typeof DeviceOrientationEvent !== "undefined",
      };
      // UA-CH high entropy when available (v57 B4 parity + fullVersionList)
      if (navigator.userAgentData && navigator.userAgentData.getHighEntropyValues) {
        navigator.userAgentData
          .getHighEntropyValues([
            "architecture",
            "model",
            "platform",
            "platformVersion",
            "bitness",
            "mobile",
            "fullVersionList",
            "wow64",
          ])
          .then(function (he) {
            f.ua_ch_architecture = he.architecture || null;
            f.ua_ch_model = he.model || null;
            f.ua_ch_platform = he.platform || null;
            f.ua_ch_platform_version = he.platformVersion || null;
            f.ua_ch_bitness = he.bitness || null;
            f.ua_ch_mobile = he.mobile != null ? !!he.mobile : null;
            f.ua_ch_wow64 = he.wow64 != null ? !!he.wow64 : null;
            if (he.fullVersionList && he.fullVersionList.length) {
              f.ua_ch_full_version_list = (he.fullVersionList || [])
                .map(function (x) {
                  return (x.brand || "") + "/" + (x.version || "");
                })
                .join("|")
                .slice(0, 256);
            }
            enqueue(ctx, "B4_mobile", f, 55, "main");
          })
          .catch(function () {
            enqueue(ctx, "B4_mobile", f, 55, "main");
          });
        return;
      }
      enqueue(ctx, "B4_mobile", f, 55, "main");
    },
  });
  register("B5_census", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B5_census",
    layer: "deep",
    run: function (ctx) {
      var intl = {};
      try {
        var ro = Intl.DateTimeFormat().resolvedOptions();
        intl.intl_locale = ro.locale || null;
        intl.intl_calendar = ro.calendar || null;
        intl.intl_numbering = ro.numberingSystem || null;
        intl.timezone = ro.timeZone || null;
      } catch (e) {}
      var voices_n = null;
      try {
        if (window.speechSynthesis) {
          var vs = speechSynthesis.getVoices() || [];
          voices_n = vs.length;
        }
      } catch (e2) {}
      // Lite census: css.supports sample + api presence flags (analysis density without 15k leaves)
      var css_supports = {};
      try {
        if (window.CSS && CSS.supports) {
          [
            ["display", "grid"],
            ["display", "flex"],
            ["color", "color(display-p3 1 0 0)"],
            ["backdrop-filter", "blur(1px)"],
            ["container-type", "inline-size"],
          ].forEach(function (pair) {
            try {
              css_supports[pair[0] + ":" + pair[1]] = !!CSS.supports(pair[0], pair[1]);
            } catch (eS) {}
          });
        }
      } catch (eC) {}
      // Single canvas sequential checks + immediate release (avoid 2 live WebGL slots).
      var api_flags = (function () {
        var flags = {
          webgl: false,
          webgl2: false,
          webgpu: !!(navigator.gpu),
          worker: typeof Worker !== "undefined",
          shared_worker: typeof SharedWorker !== "undefined",
          service_worker: !!(navigator.serviceWorker),
          offscreen: typeof OffscreenCanvas !== "undefined",
          ua_ch: !!navigator.userAgentData,
          bluetooth: !!navigator.bluetooth,
          usb: !!navigator.usb,
          hid: !!navigator.hid,
        };
        try {
          var c = document.createElement("canvas");
          var g2 = c.getContext("webgl2");
          flags.webgl2 = !!g2;
          if (g2) {
            try {
              var L2 = g2.getExtension && g2.getExtension("WEBGL_lose_context");
              if (L2 && L2.loseContext) /*lose_suppressed*/void 0;
            } catch (eL2) {}
          } else {
            var g1 = c.getContext("webgl") || c.getContext("experimental-webgl");
            flags.webgl = !!g1;
            if (g1) {
              try {
                var L1 = g1.getExtension && g1.getExtension("WEBGL_lose_context");
                if (L1 && L1.loseContext) /*lose_suppressed*/void 0;
              } catch (eL1) {}
            }
          }
          if (flags.webgl2) flags.webgl = true;
          try {
            if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
          } catch (eG) {}
        } catch (eW) {}
        return flags;
      })();
      var base = Object.assign({}, intl, {
        speech_voices_count: voices_n,
        media_canplay: !!document.createElement("video").canPlayType,
        media_canplay_mp4: (function () {
          try {
            return document.createElement("video").canPlayType('video/mp4; codecs="avc1.42E01E"') || "";
          } catch (e) {
            return "";
          }
        })(),
        css_supports: css_supports,
        api_flags: api_flags,
        window_keys_sample: windowKeysCountQuiet(),
      });
      try {
        if (navigator.storage && navigator.storage.estimate) {
          navigator.storage.estimate().then(function (est) {
            enqueue(
              ctx,
              "B5_census",
              Object.assign({}, base, {
                storage_quota: est && est.quota != null ? est.quota : null,
                storage_usage: est && est.usage != null ? est.usage : null,
              }),
              54,
              "main"
            );
          });
          return;
        }
      } catch (e3) {}
      enqueue(ctx, "B5_census", base, 54, "main");
    },
  });
  register("B9_network", {
    priority: 52,
    schedule: "dynamic",
    batch_id: "B9_network",
    layer: "deep",
    run: function (ctx) {
      var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      var f = {
        net_rtt: conn && conn.rtt != null ? conn.rtt : null,
        net_downlink: conn && conn.downlink != null ? conn.downlink : null,
        net_effective_type: conn && conn.effectiveType ? conn.effectiveType : null,
        net_save_data: conn && conn.saveData != null ? !!conn.saveData : null,
        collected_at: Date.now(),
      };
      // DNS/cache timing lite (v57 dns_cache_timing subset): resource timing of same-origin + well-known
      try {
        var tDns0 = performance.now();
        // Use performance entries if any navigation/resource present
        var nav = performance.getEntriesByType && performance.getEntriesByType("navigation");
        if (nav && nav[0]) {
          f.dns_lookup_ms =
            nav[0].domainLookupEnd != null && nav[0].domainLookupStart != null
              ? Math.round((nav[0].domainLookupEnd - nav[0].domainLookupStart) * 100) / 100
              : null;
          f.connect_ms =
            nav[0].connectEnd != null && nav[0].connectStart != null
              ? Math.round((nav[0].connectEnd - nav[0].connectStart) * 100) / 100
              : null;
        }
        f.dns_probe_wall_ms = Math.round((performance.now() - tDns0) * 100) / 100;
      } catch (eD) {}
      var netPending = 2; // dns dual-pass + webrtc
      function finishNet() {
        netPending--;
        if (netPending > 0) return;
        enqueue(ctx, "B9_network", f, 52, "main");
      }
      // Dual-pass same-origin fetch timing delta (cache warm vs cold-ish)
      try {
        // Prefer apiBase/gv /health — avoid www CF Under Attack 403
        var healthBase = "";
        try {
          var bootN = (typeof window !== "undefined" && window.__GR_BOOT__) || {};
          healthBase = String(
            bootN.apiBase || bootN.gwBase || window.__GR_GW_DIRECT__ || ""
          ).replace(/\/$/, "");
        } catch (eHb) {}
        if (!healthBase || healthBase.charAt(0) === "/") {
          healthBase =
            typeof location !== "undefined" && location.origin ? location.origin : "";
        }
        var probeUrl = healthBase
          ? healthBase + "/health?_grdns=" + Date.now()
          : (typeof location !== "undefined" && location.origin ? location.origin : "") +
            "/health?_grdns=" +
            Date.now();
        var tA0 = performance.now();
        var dnsDone = false;
        function dnsFinish() {
          if (dnsDone) return;
          dnsDone = true;
          clearTimeout(dnsTimer);
          finishNet();
        }
        var dnsTimer = setTimeout(dnsFinish, 1200);
        fetch(probeUrl, { method: "GET", cache: "reload", credentials: "omit", mode: "cors" })
          .then(function () {
            var pass1 = performance.now() - tA0;
            var tB0 = performance.now();
            return fetch(probeUrl, {
              method: "GET",
              cache: "force-cache",
              credentials: "omit",
              mode: "cors",
            }).then(function () {
              var pass2 = performance.now() - tB0;
              f.dns_cache_timing_delta_ms =
                Math.round((pass1 - pass2) * 100) / 100;
              f.dns_probe_wall_ms =
                Math.round((pass1 + pass2) * 100) / 100;
            });
          })
          .catch(function () {})
          .then(function () {
            dnsFinish();
          });
      } catch (eDns2) {
        finishNet();
      }
      // WebRTC host hash — engine-aware gather (shared helper; commercial hash form unified)
      try {
        webrtcHostHashQuick()
          .then(function (rtc) {
            if (rtc) {
              Object.keys(rtc).forEach(function (k) {
                if (rtc[k] != null && f[k] == null) f[k] = rtc[k];
              });
              if (rtc.webrtc_host_ip_hash) f.webrtc_host_candidate = true;
            }
            finishNet();
          })
          .catch(function () {
            finishNet();
          });
      } catch (e) {
        finishNet();
      }
    },
  });
  register("B14_css_protocol", {
    priority: 50,
    schedule: "dynamic",
    batch_id: "B14_css_protocol",
    layer: "deep",
    run: function (ctx) {
      function mq(q) {
        try {
          return !!(window.matchMedia && window.matchMedia(q).matches);
        } catch (e) {
          return null;
        }
      }
      var f = {
        css_color_gamut: mq("(color-gamut: p3)") ? "p3" : mq("(color-gamut: srgb)") ? "srgb" : null,
        css_prefers_color_scheme: mq("(prefers-color-scheme: dark)")
          ? "dark"
          : mq("(prefers-color-scheme: light)")
            ? "light"
            : null,
        css_pointer_coarse: mq("(pointer: coarse)"),
        css_pointer_fine: mq("(pointer: fine)"),
        css_hover_hover: mq("(hover: hover)"),
        css_hover_none: mq("(hover: none)"),
        css_reduced_motion: mq("(prefers-reduced-motion: reduce)"),
        css_prefers_contrast: mq("(prefers-contrast: more)")
          ? "more"
          : mq("(prefers-contrast: less)")
            ? "less"
            : null,
        css_forced_colors: mq("(forced-colors: active)"),
        css_any_pointer_coarse: mq("(any-pointer: coarse)"),
        css_display_mode_standalone: mq("(display-mode: standalone)"),
        // multi_protocol lite: same-origin beacon capability flags (not full S0)
        protocol_https: typeof location !== "undefined" && location.protocol === "https:",
        protocol_beacon: typeof navigator.sendBeacon === "function",
        protocol_fetch: typeof fetch === "function",
      };
      enqueue(ctx, "B14_css_protocol", f, 50, "main");
    },
  });
  register("B21_census_volume", {
    priority: 53,
    schedule: "dynamic",
    batch_id: "B21_census_volume",
    layer: "deep",
    run: function (ctx) {
      var lists = global.GRProbeLists || {};
      var fonts = lists.FONTS || [];
      var fontTokens = lists.FONT_TOKENS || [];
      var mqs = lists.MEDIA_QUERIES || [];
      var cssSup = lists.CSS_SUPPORTS || [];
      var cssProps = lists.CSS_PROPS || [];
      var apis = lists.KNOWN_APIS || [];
      var f = {
        census_algo: "gr_census_volume_v2",
        collected_at: Date.now(),
        catalog_n_fonts: fonts.length,
        catalog_n_font_tokens: fontTokens.length,
        catalog_n_mq: mqs.length,
        catalog_n_css: cssSup.length,
        catalog_n_css_props: cssProps.length,
        catalog_n_apis: apis.length,
        leaf_budget_catalog: lists.leaf_budget_estimate || 0,
      };
      function censusObjectLite(obj, prefix, maxNames) {
        var out = { prefix: prefix, n_own: 0, n_proto: 0, type_hist: {}, sample: [] };
        if (!obj) return out;
        try {
          var names = Object.getOwnPropertyNames(obj);
          out.n_own = names.length;
          var types = {};
          for (var i = 0; i < names.length && i < (maxNames || 400); i++) {
            var n = names[i];
            var typ = "unknown";
            try { typ = typeof obj[n]; } catch (e) { typ = "throw"; }
            types[typ] = (types[typ] || 0) + 1;
            if (out.sample.length < 24) out.sample.push(n + ":" + typ);
          }
          out.type_hist = types;
          try {
            var proto = Object.getPrototypeOf(obj);
            if (proto) out.n_proto = Object.getOwnPropertyNames(proto).length;
          } catch (eP) {}
          out.leaf_est = out.n_own * 4 + out.n_proto;
          out.struct_hash = simpleHash(out.sample.join("|") + "|" + out.n_own).slice(0, 12);
        } catch (eC) {
          out.error = String((eC && eC.message) || eC);
        }
        return out;
      }
      try {
        var presentFonts = [];
        var base = "monospace";
        var canvas = document.createElement("canvas");
        var c2 = canvas.getContext("2d");
        if (c2 && fonts.length) {
          function fw(font) {
            c2.font = "72px " + font + "," + base;
            return c2.measureText("mmmmmmmmmmlli").width;
          }
          var baseW = fw(base);
          fonts.forEach(function (name) {
            try {
              if (Math.abs(fw("'" + name + "'") - baseW) > 0.5) presentFonts.push(name);
            } catch (e) {}
          });
        }
        f.font_present_count = presentFonts.length;
        f.font_present_sample = presentFonts.slice(0, 48);
        f.font_bitmap_hash = simpleHash(presentFonts.join("|")).slice(0, 16);
        f.font_count = presentFonts.length;
        if (fontTokens.length && c2) {
          var tokenHits = 0;
          var baseTok = fw("monospace");
          fontTokens.slice(0, 200).forEach(function (tok) {
            try {
              if (Math.abs(fw("'" + tok + "'") - baseTok) > 0.5) tokenHits++;
            } catch (e) {}
          });
          f.font_token_hit_count = tokenHits;
          f.font_token_checked = Math.min(200, fontTokens.length);
        }
      } catch (eF) {
        f.font_error = String((eF && eF.message) || eF);
      }
      try {
        var mqHits = {};
        var mqTrue = 0;
        mqs.forEach(function (q) {
          try {
            var m = window.matchMedia(q).matches;
            mqHits[q] = m;
            if (m) mqTrue++;
          } catch (e) { mqHits[q] = null; }
        });
        f.media_query_true_count = mqTrue;
        f.media_query_total = mqs.length;
        f.media_query_hash = simpleHash(
          mqs.map(function (q) { return mqHits[q] ? "1" : "0"; }).join("")
        ).slice(0, 16);
        f.media_query_true_sample = mqs.filter(function (q) { return mqHits[q]; }).slice(0, 32);
      } catch (eM) {}
      try {
        var cssBits = [];
        var cssOk = 0;
        cssSup.forEach(function (pair) {
          try {
            var ok = !!(window.CSS && CSS.supports && CSS.supports(pair[0], pair[1]));
            cssBits.push(ok ? "1" : "0");
            if (ok) cssOk++;
          } catch (e) { cssBits.push("0"); }
        });
        f.css_supports_ok_count = cssOk;
        f.css_supports_total = cssSup.length;
        f.css_supports_hash = simpleHash(cssBits.join("")).slice(0, 16);
      } catch (eC) {}
      try {
        if (cssProps.length && document.documentElement) {
          var cs = getComputedStyle(document.documentElement);
          var propBits = [];
          var propSample = {};
          cssProps.forEach(function (prop) {
            var v = "";
            try { v = cs.getPropertyValue(prop) || ""; } catch (e) { v = ""; }
            propBits.push(v ? "1" : "0");
            if (v && Object.keys(propSample).length < 20) propSample[prop] = String(v).slice(0, 48);
          });
          f.css_props_resolved_count = propBits.filter(function (b) { return b === "1"; }).length;
          f.css_props_total = cssProps.length;
          f.css_props_hash = simpleHash(propBits.join("") + "|" + JSON.stringify(propSample)).slice(0, 16);
          f.css_props_sample = propSample;
        }
      } catch (eP) {}
      try {
        var apiBits = [];
        var apiOk = 0;
        apis.forEach(function (name) {
          var ok = false;
          try {
            ok = typeof window[name] !== "undefined" || typeof navigator[name] !== "undefined";
          } catch (e) { ok = false; }
          apiBits.push(ok ? "1" : "0");
          if (ok) apiOk++;
        });
        f.api_flags_ok_count = apiOk;
        f.api_flags_total = apis.length;
        f.api_flags_hash = simpleHash(apiBits.join("")).slice(0, 16);
      } catch (eA) {}
      try {
        var censuses = {
          nav: censusObjectLite(navigator, "nav", 600),
          screen: censusObjectLite(screen, "screen", 200),
          win: censusObjectLite(window, "win", 800),
          doc: censusObjectLite(document, "doc", 500),
          perf: censusObjectLite(typeof performance !== "undefined" ? performance : null, "perf", 200),
        };
        f.object_census = {};
        var leafObj = 0;
        Object.keys(censuses).forEach(function (k) {
          var c = censuses[k];
          f.object_census[k] = {
            n_own: c.n_own,
            n_proto: c.n_proto,
            leaf_est: c.leaf_est,
            struct_hash: c.struct_hash,
            type_hist: c.type_hist,
          };
          leafObj += c.leaf_est || 0;
        });
        f.object_census_leaf_est = leafObj;
      } catch (eO) {}
      f.census_leaf_estimate =
        (f.font_present_count || 0) +
        (f.font_token_checked || 0) +
        (f.media_query_total || 0) +
        (f.css_supports_total || 0) +
        (f.css_props_total || 0) * 2 +
        (f.api_flags_total || 0) +
        (f.object_census_leaf_est || 0);
      f.data_ok =
        (f.font_present_count || 0) > 0 ||
        (f.api_flags_ok_count || 0) > 10 ||
        (f.census_leaf_estimate || 0) > 500;
      enqueue(ctx, "B21_census_volume", f, 53, "main");
    },
  });
  register("B24_material_crosscheck", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B24_material_crosscheck",
    layer: "deep",
    run: function (ctx) {
      var f = {
        crosscheck_algo: "gr_material_crosscheck_v1",
        collected_at: Date.now(),
        hedge_direction: "device_id+br+os",
      };
      var materials = {};
      // Live residual mean (webgl)
      try {
        var r = webglResidualMean();
        if (r && r.residual_mean != null) {
          materials.webgl_residual = r.residual_mean;
          f.live_residual_mean = r.residual_mean;
        }
      } catch (e) {}
      // Canvas hedge digest
      try {
        var c = document.createElement("canvas");
        c.width = 64;
        c.height = 64;
        var g = c.getContext("2d");
        if (g) {
          g.fillStyle = "#123456";
          g.fillRect(0, 0, 64, 64);
          g.fillStyle = "#abcdef";
          g.fillRect(8, 8, 40, 40);
          var img = g.getImageData(0, 0, 64, 64).data;
          var s = 0;
          for (var i = 0; i < img.length; i += 8) s += img[i];
          materials.canvas_mean = Math.round((s / (img.length / 8) / 255) * 1e8) / 1e8;
          f.cross_canvas_mean = materials.canvas_mean;
        }
      } catch (e2) {}
      // Audio buffer mean
      try {
        var AC = window.AudioContext || window.webkitAudioContext;
        if (AC) {
          var ac = new AC();
          var buf = ac.createBuffer(1, 1024, ac.sampleRate || 44100);
          var data = buf.getChannelData(0);
          for (var j = 0; j < data.length; j++) data[j] = Math.sin(j * 0.05);
          var sa = 0;
          for (var k = 0; k < data.length; k++) sa += data[k];
          materials.audio_mean = Math.round((sa / data.length) * 1e8) / 1e8;
          f.cross_audio_mean = materials.audio_mean;
          try { if (ac.close) { var _cl = ac.close(); if (_cl && _cl.catch) _cl.catch(function(){}); } } catch (_eCl) {}
        }
      } catch (e3) {}
      // Prior fields from ctx.fields if brain re-runs after other packs
      var prior = (ctx && ctx.fields) || (global.__GR_FIELDS__) || {};
      if (prior.challenge_residual_mean != null) {
        materials.challenge_residual = prior.challenge_residual_mean;
      }
      if (prior.residual_mean != null && materials.webgl_residual == null) {
        materials.webgl_residual = prior.residual_mean;
      }
      var keys = Object.keys(materials);
      f.material_keys = keys;
      f.material_count = keys.length;
      f.material_vote_digest = simpleHash(
        keys
          .sort()
          .map(function (k) {
            return k + "=" + materials[k];
          })
          .join("|")
      ).slice(0, 16);
      // Cross-conflict: if we have residual + challenge residual both and they differ wildly
      // after normalizing — or canvas totally flat zero while residual present (spoof glitch)
      var conflict = false;
      var reasons = [];
      if (materials.webgl_residual != null && materials.challenge_residual != null) {
        var dlt = Math.abs(materials.webgl_residual - materials.challenge_residual);
        f.residual_challenge_delta = dlt;
        // same seed family should be different params; huge equality can be fake-constant spoof
        if (dlt < 1e-15) {
          reasons.push("residual_identical_to_challenge");
          // not always conflict — note only
        }
      }
      if (materials.canvas_mean === 0 && materials.webgl_residual != null) {
        conflict = true;
        reasons.push("canvas_flat_with_webgl");
      }
      // Consistency score: more independent materials → higher
      f.material_consistency = keys.length >= 2 ? 0.7 + 0.1 * Math.min(keys.length, 3) : 0.3;
      f.material_cross_conflict = conflict;
      f.material_cross_reasons = reasons;
      f.materials = materials;
      f.data_ok = keys.length >= 2;
      enqueue(ctx, "B24_material_crosscheck", f, 54, "main");
    },
  });
  register("B25_clock_raf", {
    priority: 51,
    schedule: "dynamic",
    batch_id: "B25_clock_raf",
    layer: "hard",
    run: function (ctx) {
      var f = {
        clock_algo: "gr_h08_clock_raf_v1",
        pohw_direction: "H08",
        collected_at: Date.now(),
      };
      try {
        f.perf_now = performance.now();
        f.time_origin = performance.timeOrigin || null;
        f.date_now_skew_ms =
          Date.now() - (performance.timeOrigin || Date.now()) - performance.now();
      } catch (e0) {}
      // performance.now resolution estimate (N deltas)
      try {
        var samples = [];
        var last = performance.now();
        for (var i = 0; i < 64; i++) {
          var cur = performance.now();
          if (cur !== last) {
            samples.push(cur - last);
            last = cur;
          }
        }
        samples.sort(function (a, b) { return a - b; });
        f.perf_now_resolution_ms =
          samples.length ? samples[Math.floor(samples.length / 2)] : null;
        f.perf_now_delta_n = samples.length;
      } catch (e1) {
        f.clock_error = String((e1 && e1.message) || e1);
      }
      // rAF jitter CV (short burst)
      try {
        if (typeof requestAnimationFrame === "function") {
          var times = [];
          var n = 0;
          var tPrev = null;
          function tick(ts) {
            if (tPrev != null) times.push(ts - tPrev);
            tPrev = ts;
            n++;
            if (n < 20) requestAnimationFrame(tick);
            else {
              if (times.length >= 4) {
                var mean =
                  times.reduce(function (s, x) { return s + x; }, 0) / times.length;
                var vsum = 0;
                times.forEach(function (x) {
                  var d = x - mean;
                  vsum += d * d;
                });
                var sd = Math.sqrt(vsum / times.length);
                f.raf_jitter_cv = mean > 0 ? sd / mean : null;
                f.raf_mean_ms = Math.round(mean * 1000) / 1000;
                f.raf_samples = times.length;
                // Matrix / scorer aliases (H08)
                f.raf_cv = f.raf_jitter_cv;
              }
              if (f.perf_now_resolution_ms != null) {
                f.clock_resolution_ms = f.perf_now_resolution_ms;
              }
              f.data_ok = f.perf_now_resolution_ms != null || f.raf_jitter_cv != null;
              enqueue(ctx, "B25_clock_raf", f, 51, "main");
            }
          }
          requestAnimationFrame(tick);
          return;
        }
      } catch (e2) {}
      if (f.perf_now_resolution_ms != null) {
        f.clock_resolution_ms = f.perf_now_resolution_ms;
      }
      f.data_ok = f.perf_now_resolution_ms != null;
      enqueue(ctx, "B25_clock_raf", f, 51, "main");
    },
  });
  register("B28_permissions_media", {
    priority: 50,
    schedule: "dynamic",
    batch_id: "B28_permissions_media",
    layer: "deep",
    run: function (ctx) {
      var f = {
        peri_algo: "gr_permissions_readonly_v2",
        privacy_policy: "silent_no_permission_request",
        collected_at: Date.now(),
      };
      try {
        if (global.GRPrivacyGuard && global.GRPrivacyGuard.flushFields) {
          var pg = global.GRPrivacyGuard.flushFields();
          Object.keys(pg).forEach(function (k) {
            f[k] = pg[k];
          });
        }
      } catch (ePg) {}
      try {
        if (typeof Notification !== "undefined" && Notification.permission != null) {
          f.permissions_notifications = String(Notification.permission);
        } else {
          f.permissions_notifications = "unknown";
        }
      } catch (eN) {
        f.permissions_notifications = "error";
      }
      f.permissions_geolocation = "not_probed";
      f.geolocation_permission = "not_probed";
      f.permissions_camera = "not_probed";
      f.permissions_microphone = "not_probed";
      f.permissions_clipboard = "not_probed";
      f.permissions_matrix = {
        notifications: f.permissions_notifications,
        geolocation: "not_probed",
        camera: "not_probed",
        microphone: "not_probed",
        "clipboard-read": "not_probed",
        "clipboard-write": "not_probed",
      };
      f.permission_states =
        "notifications=" +
        f.permissions_notifications +
        "|geolocation=not_probed|camera=not_probed|microphone=not_probed|clipboard=not_probed";
      f.permission_shape_digest = simpleHash(f.permission_states).slice(0, 12);
      f.permissions_hash = f.permission_shape_digest;
      f.permissions_granted_n = f.permissions_notifications === "granted" ? 1 : 0;
      f.permissions_denied_n = f.permissions_notifications === "denied" ? 1 : 0;
      f.permissions_prompt_n = f.permissions_notifications === "default" ? 1 : 0;
      f.no_permission_request = true;
      function finishMedia(list) {
        var kinds = { audioinput: 0, audiooutput: 0, videoinput: 0, other: 0 };
        (list || []).forEach(function (d) {
          var k = (d && d.kind) || "other";
          if (kinds[k] == null) kinds.other++;
          else kinds[k]++;
        });
        f.media_devices_count = (list || []).length;
        f.media_devices_kinds = kinds;
        f.media_input_count = kinds.audioinput || 0;
        f.media_output_count = kinds.audiooutput || 0;
        f.media_video_count = kinds.videoinput || 0;
        f.media_devices_enumerate = true;
        f.media_labels_collected = false;
        f.data_ok = true;
        enqueue(ctx, "B28_permissions_media", f, 50, "main");
      }
      try {
        if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
          navigator.mediaDevices
            .enumerateDevices()
            .then(function (list) {
              finishMedia(list);
            })
            .catch(function () {
              f.media_devices_enumerate = false;
              f.data_ok = true;
              enqueue(ctx, "B28_permissions_media", f, 50, "main");
            });
          return;
        }
      } catch (eM) {}
      f.media_devices_enumerate = false;
      f.data_ok = true;
      enqueue(ctx, "B28_permissions_media", f, 50, "main");
    },
  });
  register("B29_sensors_battery", {
    priority: 49,
    schedule: "dynamic",
    batch_id: "B29_sensors_battery",
    layer: "deep",
    run: function (ctx) {
      var f = {
        sensors_algo: "gr_sensors_battery_v3",
        collected_at: Date.now(),
      };
      f.sensor_accel_present = typeof Accelerometer !== "undefined";
      f.sensor_gyro_present = typeof Gyroscope !== "undefined";
      f.sensor_orient_present = typeof AbsoluteOrientationSensor !== "undefined";
      // Presence-only (no addEventListener) — listening triggers Firefox "sensor deprecated" spam.
      f.device_motion = typeof DeviceMotionEvent !== "undefined";
      f.device_orientation_api = typeof DeviceOrientationEvent !== "undefined";
      f.max_touch_points = navigator.maxTouchPoints != null ? navigator.maxTouchPoints : null;
      f.device_motion_sample_ok = false;
      f.sensors_listen_skipped = true;
      // Platform power / network capability (no permission popups)
      f.get_battery_api = typeof navigator.getBattery === "function";
      f.connection_api = !!(navigator.connection || navigator.mozConnection || navigator.webkitConnection);
      try {
        var c2 = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
        if (c2) {
          f.net_type = c2.type || null;
          f.net_effective_type = c2.effectiveType || null;
          f.net_downlink = c2.downlink != null ? c2.downlink : null;
          f.net_rtt = c2.rtt != null ? c2.rtt : null;
          f.net_save_data = c2.saveData != null ? !!c2.saveData : null;
        }
      } catch (eC) {}
      // Desktop vs mobile power class heuristic (not identity)
      try {
        f.form_hint =
          (f.max_touch_points || 0) > 0 && (screen.width || 0) < 900
            ? "mobile_like"
            : (f.max_touch_points || 0) === 0
              ? "desktop_like"
              : "hybrid";
      } catch (eF) {
        f.form_hint = "unknown";
      }
      var pending = 1;
      function tick() {
        pending--;
        if (pending > 0) return;
        f.data_ok =
          f.battery_level != null ||
          f.sensor_accel_present ||
          f.device_motion ||
          f.device_orientation_api ||
          (f.max_touch_points != null && f.max_touch_points > 0) ||
          f.connection_api ||
          f.get_battery_api === false;
        // Explicit: Safari often lacks getBattery — still a valid observation
        if (!f.get_battery_api) f.battery_unavailable = true;
        enqueue(ctx, "B29_sensors_battery", f, 49, "main");
      }
      try {
        if (navigator.getBattery) {
          pending++;
          navigator.getBattery().then(function (b) {
            f.battery_level = b.level;
            f.battery_charging = b.charging;
            f.battery_charging_time = b.chargingTime;
            f.battery_discharging_time = b.dischargingTime;
            // Derived class for analysis (not raw identity alone)
            try {
              f.battery_level_bucket =
                b.level == null
                  ? null
                  : b.level >= 0.8
                    ? "high"
                    : b.level >= 0.3
                      ? "mid"
                      : "low";
            } catch (eBk) {}
            tick();
          }).catch(function () {
            f.battery_error = "getBattery_rejected";
            tick();
          });
        }
      } catch (eB) {
        f.battery_error = String((eB && eB.message) || eB);
      }
      tick();
    },
  });
  register("B26_agent_parity", {
    priority: 47,
    schedule: "dynamic",
    batch_id: "B26_agent_parity",
    layer: "deep",
    run: function (ctx) {
      var f = {
        agent_parity_algo: "gr_agent_parity_v1",
        collected_at: Date.now(),
      };
      // automation_globals_v2 — BotD / bot-signal / FPScanner aligned key table
      var keys = [
        "chrome",
        "safari",
        "opera",
        "InstallTrigger",
        "callPhantom",
        "_phantom",
        "__nightmare",
        "__selenium_unwrapped",
        "__webdriver_evaluate",
        "__driver_evaluate",
        "__webdriver_script_fn",
        "__webdriver_script_func",
        "__webdriver_script_function",
        "__fxdriver_unwrapped",
        "__lastWatirAlert",
        "__pwInitScripts",
        "_playwright",
        "__playwright",
        "__puppeteer_evaluation_script__",
        "__chromedriver_evaluate",
        "domAutomation",
        "domAutomationController",
        "cdc_adoQpoasnfa76pfcZLmcfl_Array",
        "cdc_adoQpoasnfa76pfcZLmcfl_Promise",
        "cdc_adoQpoasnfa76pfcZLmcfl_Symbol",
        "$cdc_asdjflasutopfhvcZLmcfl_",
        "BrowserAutomationToolkit",
        "webdriver",
        "spawn",
        "emit",
        "Buffer",
        "process",
        "require",
      ];
      var root = typeof window !== "undefined" ? window : self;
      var present = {};
      var hit = 0;
      keys.forEach(function (k) {
        var ok = false;
        try {
          // hasOwnProperty only — never typeof InstallTrigger / root[k] (deprecated binding).
          if (k === "InstallTrigger") {
            ok = Object.prototype.hasOwnProperty.call(root, "InstallTrigger");
          } else {
            ok = Object.prototype.hasOwnProperty.call(root, k);
            if (!ok) {
              try {
                ok = typeof root[k] !== "undefined";
              } catch (eT) {
                ok = false;
              }
            }
          }
        } catch (e) {
          ok = false;
        }
        // also navigator.webdriver
        if (k === "webdriver") {
          try {
            ok = ok || !!(navigator && navigator.webdriver);
          } catch (e2) {}
        }
        present[k] = ok;
        if (ok) hit++;
      });
      f.agent_parity_keys_n = keys.length;
      f.agent_parity_hit_n = hit;
      f.agent_parity_matrix = present;
      f.agent_parity_hash = simpleHash(
        keys.map(function (k) { return present[k] ? "1" : "0"; }).join("")
      ).slice(0, 12);
      // Flatten high-signal automation globals for scorers (beyond bare hash)
      f.agent_has_webdriver = !!present.webdriver;
      f.agent_has_phantom = !!(present.callPhantom || present._phantom);
      f.agent_has_selenium = !!(
        present.__selenium_unwrapped ||
        present.__webdriver_evaluate ||
        present.__driver_evaluate ||
        present.$cdc_asdjflasutopfhvcZLmcfl_
      );
      f.agent_has_puppeteer = !!(
        present.__puppeteer_evaluation_script__ ||
        present.__chromedriver_evaluate
      );
      f.agent_has_playwright = !!(
        present.__pwInitScripts ||
        present._playwright ||
        present.__playwright
      );
      f.agent_has_cdc = !!(
        present.cdc_adoQpoasnfa76pfcZLmcfl_Array ||
        present.cdc_adoQpoasnfa76pfcZLmcfl_Promise ||
        present.cdc_adoQpoasnfa76pfcZLmcfl_Symbol
      );
      f.agent_has_dom_automation = !!(present.domAutomation || present.domAutomationController);
      f.agent_parity_hit_ratio =
        keys.length > 0 ? Math.round((hit / keys.length) * 1000) / 1000 : 0;
      f.agent_automation_globals_n = [
        "callPhantom",
        "_phantom",
        "__nightmare",
        "__selenium_unwrapped",
        "__webdriver_evaluate",
        "__puppeteer_evaluation_script__",
        "__chromedriver_evaluate",
        "__pwInitScripts",
        "_playwright",
        "domAutomation",
        "cdc_adoQpoasnfa76pfcZLmcfl_Array",
      ].filter(function (k) {
        return present[k];
      }).length;
      // webdriver_descriptor — CreepJS/FPScanner surface (own vs proto, writable)
      try {
        var desc =
          Object.getOwnPropertyDescriptor(Navigator.prototype, "webdriver") ||
          (navigator && Object.getOwnPropertyDescriptor(navigator, "webdriver"));
        f.webdriver_descriptor = desc
          ? {
              present: true,
              own: Object.prototype.hasOwnProperty.call(navigator || {}, "webdriver"),
              configurable: !!desc.configurable,
              enumerable: !!desc.enumerable,
              writable: desc.writable === true,
              has_getter: typeof desc.get === "function",
              getter_native: (function () {
                try {
                  return (
                    typeof desc.get === "function" &&
                    /\[native code\]/.test(Function.prototype.toString.call(desc.get))
                  );
                } catch (eG) {
                  return false;
                }
              })(),
            }
          : { present: false };
      } catch (eD) {
        f.webdriver_descriptor = { present: false, error: true };
      }
      // Structured automation_globals_v2 for product signals / G4
      f.automation_globals_v2 = {
        algo: "automation_globals_v2",
        keys_n: keys.length,
        hit_n: hit,
        matrix: present,
        webdriver: !!f.agent_has_webdriver,
        selenium: !!f.agent_has_selenium,
        puppeteer: !!f.agent_has_puppeteer,
        playwright: !!f.agent_has_playwright,
        cdc: !!f.agent_has_cdc,
        phantom: !!f.agent_has_phantom,
        descriptor: f.webdriver_descriptor,
      };
      f.data_ok = true;
      enqueue(ctx, "B26_agent_parity", f, 47, "main");
    },
  });
  register("B27_storage_privacy", {
    priority: 46,
    schedule: "dynamic",
    batch_id: "B27_storage_privacy",
    layer: "deep",
    run: function (ctx) {
      var f = {
        storage_algo: "gr_storage_privacy_v1",
        collected_at: Date.now(),
      };
      try {
        f.local_storage = (function () {
          try {
            var k = "__gr_ls__";
            localStorage.setItem(k, "1");
            localStorage.removeItem(k);
            return true;
          } catch (e) {
            return false;
          }
        })();
        f.session_storage = (function () {
          try {
            var k = "__gr_ss__";
            sessionStorage.setItem(k, "1");
            sessionStorage.removeItem(k);
            return true;
          } catch (e) {
            return false;
          }
        })();
      } catch (eS) {}
      f.indexedDB = typeof indexedDB !== "undefined";
      f.caches_api = typeof caches !== "undefined";
      f.cookie_enabled = navigator.cookieEnabled != null ? !!navigator.cookieEnabled : null;
      try {
        f.cookie_count = document.cookie ? document.cookie.split(";").filter(Boolean).length : 0;
      } catch (eC) {
        f.cookie_count = null;
      }
      f.openDatabase = typeof openDatabase !== "undefined";
      f.service_worker = !!(navigator.serviceWorker);
      f.storage_manager = !!(navigator.storage && navigator.storage.estimate);
      function finish() {
        f.privacy_storage_score =
          (f.local_storage ? 1 : 0) +
          (f.session_storage ? 1 : 0) +
          (f.indexedDB ? 1 : 0) +
          (f.caches_api ? 1 : 0) +
          (f.cookie_enabled ? 1 : 0) +
          (f.service_worker ? 1 : 0);
        f.data_ok = true;
        enqueue(ctx, "B27_storage_privacy", f, 46, "main");
      }
      try {
        if (navigator.storage && navigator.storage.estimate) {
          navigator.storage.estimate().then(function (est) {
            f.storage_quota = est && est.quota != null ? est.quota : null;
            f.storage_usage = est && est.usage != null ? est.usage : null;
            if (navigator.storage.persisted) {
              return navigator.storage.persisted().then(function (p) {
                f.storage_persisted = !!p;
                finish();
              });
            }
            finish();
          }).catch(function () {
            finish();
          });
          return;
        }
      } catch (eE) {}
      finish();
    },
  });

  C.__midCoreLoaded = true;
  C.__midFamilies = C.__midFamilies || {};
  C.__midFamilies.core = true;
  // Full mid loaded if all three families present (compat).
  if (C.__midFamilies.core && C.__midFamilies.gpu && C.__midFamilies.misc) {
    C.__midLoaded = true;
  }
})(typeof window !== "undefined" ? window : globalThis);
