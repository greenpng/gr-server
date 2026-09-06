/* green-v5 registry.mid.misc — auto-split from registry.js; do not edit by hand */
(function (global) {
  "use strict";
  var C = global.GRCollectors;
  if (!C || !C.register) {
    try { console.warn("[gr] registry.mid.misc: static registry not loaded"); } catch (e) {}
    return;
  }
  if (C.__midMiscLoaded || (C.get && C.get("B35_dom_perf"))) {
    C.__midMiscLoaded = true;
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
    try { console.warn("[gr] registry.mid.misc: helpers missing from static"); } catch (e2) {}
    return;
  }

  register("B84_gpu_bandwidth_ladder", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B84_gpu_bandwidth_ladder",
    layer: "hard",
    run: function (ctx) {
      // Reuse B30 implementation via forced re-run with pack id tag
      var f = {
        bw_algo: "gr_a4_bandwidth_ladder_v1",
        pohw_direction: "A4",
        collected_at: Date.now(),
      };
      try {
        var c = document.createElement("canvas");
        var gl =
          c.getContext("webgl", { preserveDrawingBuffer: true }) ||
          c.getContext("experimental-webgl");
        if (!gl) {
          f.bw_skip = "no_webgl";
          f.data_ok = false;
          enqueue(ctx, "B84_gpu_bandwidth_ladder", f, 54, "main");
          return;
        }
        var sizes = [64, 128, 256, 512, 1024, 2048];
        var readback = [];
        sizes.forEach(function (sz) {
          c.width = sz;
          c.height = sz;
          gl.viewport(0, 0, sz, sz);
          gl.clearColor(0.05, 0.1, 0.15, 1);
          gl.clear(gl.COLOR_BUFFER_BIT);
          var out = new Uint8Array(sz * sz * 4);
          var t1 = performance.now();
          gl.readPixels(0, 0, sz, sz, gl.RGBA, gl.UNSIGNED_BYTE, out);
          gl.finish();
          var rbMs = performance.now() - t1;
          var bytes = sz * sz * 4;
          readback.push({
            size: sz,
            wall_ms: Math.round(rbMs * 1000) / 1000,
            mib_s: rbMs > 0 ? Math.round((bytes / (rbMs / 1000) / (1024 * 1024)) * 100) / 100 : null,
          });
        });
        f.gpu_readback_ladder = readback;
        f.gpu_bandwidth_ladder = readback;
        if (readback.length >= 3) {
          var xs = readback.map(function (r) { return r.size; });
          var ys = readback.map(function (r) { return r.wall_ms; });
          var n = xs.length, sx = 0, sy = 0, sxx = 0, sxy = 0, i;
          for (i = 0; i < n; i++) {
            sx += xs[i]; sy += ys[i]; sxx += xs[i] * xs[i]; sxy += xs[i] * ys[i];
          }
          var den = n * sxx - sx * sx;
          f.gpu_wall_staircase_slope = den ? (n * sxy - sx * sy) / den : null;
          f.roundtrip_slope = f.gpu_wall_staircase_slope;
        }
        f.data_ok = readback.length >= 4;
      } catch (e) {
        f.bw_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B84_gpu_bandwidth_ladder", f, 54, "main");
    },
  });
  register("B35_dom_perf", {
    priority: 45,
    schedule: "dynamic",
    batch_id: "B35_dom_perf",
    layer: "deep",
    run: function (ctx) {
      var f = {
        dom_perf_algo: "gr_dom_perf_v1",
        collected_at: Date.now(),
      };
      // DomRect probe
      try {
        if (typeof document !== "undefined" && document.body) {
          var d = document.createElement("div");
          d.style.cssText =
            "position:absolute;left:-9999px;top:0;width:100.5px;height:33.3px;font:16px Arial;padding:0;margin:0;";
          d.textContent = "mmmmmmmmmmlli";
          document.body.appendChild(d);
          var r = d.getBoundingClientRect();
          f.dom_rect = {
            width: Math.round(r.width * 1000) / 1000,
            height: Math.round(r.height * 1000) / 1000,
            x: Math.round(r.x * 1000) / 1000,
            y: Math.round(r.y * 1000) / 1000,
          };
          f.dom_rect_hash = simpleHash(
            [f.dom_rect.width, f.dom_rect.height, f.dom_rect.x, f.dom_rect.y].join(",")
          ).slice(0, 12);
          // subpixel detection
          f.dom_rect_subpixel = Math.abs(r.width - Math.round(r.width)) > 1e-6;
          document.body.removeChild(d);
        } else {
          f.dom_rect_skip = "no_document";
        }
      } catch (eD) {
        f.dom_rect_error = String((eD && eD.message) || eD);
      }
      // Performance timing surface
      try {
        f.perf_time_origin = performance.timeOrigin || null;
        f.perf_now = performance.now();
        if (performance.timing) {
          var t = performance.timing;
          f.perf_nav_timing = {
            dom_complete_ms: t.domComplete && t.navigationStart ? t.domComplete - t.navigationStart : null,
            load_event_ms: t.loadEventEnd && t.navigationStart ? t.loadEventEnd - t.navigationStart : null,
            response_start_ms:
              t.responseStart && t.navigationStart ? t.responseStart - t.navigationStart : null,
          };
        }
        // PerformanceObserver entry counts (snapshot)
        if (performance.getEntriesByType) {
          var types = ["navigation", "resource", "paint", "measure", "mark"];
          f.perf_entry_counts = {};
          var supported =
            typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes
              ? PerformanceObserver.supportedEntryTypes
              : null;
          types.forEach(function (ty) {
            try {
              if (supported && supported.indexOf(ty) < 0) {
                f.perf_entry_counts[ty] = null;
                return;
              }
              f.perf_entry_counts[ty] = (performance.getEntriesByType(ty) || []).length;
            } catch (e) {
              f.perf_entry_counts[ty] = null;
            }
          });
          f.perf_timeline_hash = simpleHash(JSON.stringify(f.perf_entry_counts)).slice(0, 12);
        }
        // long task support
        f.perf_longtask_observer =
          typeof PerformanceObserver !== "undefined" &&
          (function () {
            try {
              return PerformanceObserver.supportedEntryTypes
                ? PerformanceObserver.supportedEntryTypes.indexOf("longtask") >= 0
                : false;
            } catch (e) {
              return false;
            }
          })();
      } catch (eP) {
        f.perf_error = String((eP && eP.message) || eP);
      }
      f.data_ok = !!(f.dom_rect_hash || f.perf_timeline_hash || f.perf_nav_timing);
      enqueue(ctx, "B35_dom_perf", f, 45, "main");
    },
  });
  register("B38_neg_dict", {
    priority: 43,
    schedule: "dynamic",
    batch_id: "B38_neg_dict",
    layer: "deep",
    run: function (ctx) {
      var f = {
        neg_dict_algo: "gr_h15_neg_dict_v1",
        pohw_direction: "H15",
        collected_at: Date.now(),
      };
      var hits = [];
      function hit(code, ok) {
        if (ok) hits.push(code);
      }
      try {
        hit("webdriver", !!navigator.webdriver);
        hit("outer_zero", window.outerWidth === 0 && window.outerHeight === 0);
        hit("plugins_empty", !navigator.plugins || navigator.plugins.length === 0);
        hit("languages_empty", !navigator.languages || navigator.languages.length === 0);
        hit("chrome_missing", typeof window.chrome === "undefined" && /Chrome\//.test(navigator.userAgent || ""));
        hit("permission_denied_all", false); // filled if permissions API bulk-denied later
        hit("callPhantom", !!(window.callPhantom || window._phantom));
        hit("selenium", !!(window.__selenium_unwrapped || window._Selenium_IDE_Recorder));
        hit("puppeteer", !!(window.__puppeteer_evaluation_script__));
        hit("cdc_prop", Object.keys(window).some(function (k) { return k.indexOf("cdc_") === 0; }));
        hit("headless_ua", /HeadlessChrome|PhantomJS|Electron/i.test(navigator.userAgent || ""));
        hit("webgl_soft_label", (function () {
          try {
            var c = document.createElement("canvas");
            var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
            if (!gl) return true;
            var dbg = gl.getExtension("WEBGL_debug_renderer_info");
            var r = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : "";
            return /swiftshader|llvmpipe|softpipe|basic render/i.test(String(r));
          } catch (e) {
            return false;
          }
        })());
      } catch (eN) {
        f.neg_error = String((eN && eN.message) || eN);
      }
      f.neg_dict_hits = hits;
      f.neg_dict_hit_n = hits.length;
      f.neg_dict_hash = simpleHash(hits.sort().join("|")).slice(0, 12);
      f.data_ok = true;
      enqueue(ctx, "B38_neg_dict", f, 43, "main");
    },
  });
  register("B39_mem_pressure", {
    priority: 42,
    schedule: "dynamic",
    batch_id: "B39_mem_pressure",
    layer: "deep",
    run: function (ctx) {
      var f = {
        mem_algo: "gr_d08_mem_pressure_v1",
        collected_at: Date.now(),
        device_memory: navigator.deviceMemory != null ? navigator.deviceMemory : null,
        hardware_concurrency: navigator.hardwareConcurrency != null ? navigator.hardwareConcurrency : null,
      };
      try {
        if (performance && performance.memory) {
          f.js_heap_size_limit = performance.memory.jsHeapSizeLimit;
          f.total_js_heap_size = performance.memory.totalJSHeapSize;
          f.used_js_heap_size = performance.memory.usedJSHeapSize;
        }
      } catch (eM) {}
      // allocation ladder until soft fail
      var sizes = [1, 2, 4, 8, 16, 32]; // MB attempts (small)
      var ladder = [];
      var maxOk = 0;
      for (var i = 0; i < sizes.length; i++) {
        var mb = sizes[i];
        try {
          var n = (mb * 1024 * 1024) / 4;
          var t0 = performance.now();
          var buf = new Float32Array(n);
          buf[0] = 1;
          buf[buf.length - 1] = 2;
          var ms = performance.now() - t0;
          ladder.push({ mb: mb, ok: true, alloc_ms: Math.round(ms * 1000) / 1000 });
          maxOk = mb;
          buf = null;
        } catch (eA) {
          ladder.push({ mb: mb, ok: false, err: String((eA && eA.message) || eA) });
          break;
        }
      }
      f.mem_alloc_ladder = ladder;
      f.mem_alloc_max_mb = maxOk;
      f.data_ok = ladder.length > 0;
      enqueue(ctx, "B39_mem_pressure", f, 42, "main");
    },
  });
  register("B40_websocket_fp", {
    priority: 41,
    schedule: "dynamic",
    batch_id: "B40_websocket_fp",
    layer: "deep",
    run: function (ctx) {
      var f = {
        ws_algo: "gr_d21_websocket_fp_v1",
        collected_at: Date.now(),
        websocket_present: typeof WebSocket !== "undefined",
      };
      if (typeof WebSocket === "undefined") {
        f.data_ok = false;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
        return;
      }
      try {
        // Constructor fingerprint only — no live /gr-ws-probe (nginx returns 200, floods console).
        f.ws_constructor_native = /\[native code\]/.test(Function.prototype.toString.call(WebSocket));
        f.ws_binary_types = null;
        try {
          f.ws_proto_keys_n = Object.getOwnPropertyNames(WebSocket.prototype || {}).length;
        } catch (ePk) {
          f.ws_proto_keys_n = null;
        }
        f.ws_url_host = typeof location !== "undefined" ? location.host : null;
        f.ws_probe_mode = "constructor_only";
        f.ws_connect_skipped = "no_ws_upgrade_endpoint";
        f.ws_close_reason = "skipped_no_endpoint";
        f.data_ok = true;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
      } catch (e) {
        f.ws_error = String((e && e.message) || e);
        f.data_ok = true;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
      }
    },
  });
  register("B41_hid_gamepad", {
    priority: 40,
    schedule: "dynamic",
    batch_id: "B41_hid_gamepad",
    layer: "deep",
    run: function (ctx) {
      var f = {
        hid_algo: "gr_d35_hid_gamepad_v1",
        collected_at: Date.now(),
        hid_api: !!(navigator.hid),
        usb_api: !!(navigator.usb),
        serial_api: !!(navigator.serial),
        bluetooth_api: !!(navigator.bluetooth),
        gamepad_api: typeof navigator.getGamepads === "function",
      };
      try {
        if (f.gamepad_api) {
          var gps = navigator.getGamepads() || [];
          var n = 0;
          var ids = [];
          for (var i = 0; i < gps.length; i++) {
            if (gps[i]) {
              n++;
              ids.push(String(gps[i].id || "").slice(0, 64));
            }
          }
          f.gamepad_count = n;
          f.gamepad_ids_sample = ids.slice(0, 4);
        }
      } catch (eG) {
        f.gamepad_error = String((eG && eG.message) || eG);
      }
      f.hid_surface_score =
        (f.hid_api ? 1 : 0) +
        (f.usb_api ? 1 : 0) +
        (f.serial_api ? 1 : 0) +
        (f.bluetooth_api ? 1 : 0) +
        (f.gamepad_api ? 1 : 0);
      f.data_ok = true;
      enqueue(ctx, "B41_hid_gamepad", f, 40, "main");
    },
  });
  register("B43_errors_engine", {
    priority: 39,
    schedule: "dynamic",
    batch_id: "B43_errors_engine",
    layer: "deep",
    run: function (ctx) {
      var f = {
        errors_algo: "gr_errors_engine_v1",
        collected_at: Date.now(),
      };
      function catchShape(fn) {
        try {
          fn();
          return { threw: false };
        } catch (e) {
          return {
            threw: true,
            name: e && e.name ? String(e.name) : "Error",
            message: e && e.message ? String(e.message).slice(0, 120) : "",
            stack_head: e && e.stack ? String(e.stack).split("\\n").slice(0, 3).join("|").slice(0, 200) : "",
          };
        }
      }
      f.errors_engine = {
        null_prop: catchShape(function () { return null.x; }),
        undef_call: catchShape(function () { return undefined(); }),
        object_call: catchShape(function () { return ({})(); }),
        new_number: catchShape(function () { return new (1)(); }),
      };
      // Engine-specific strings
      var msgs = Object.keys(f.errors_engine).map(function (k) {
        var e = f.errors_engine[k];
        return e.threw ? e.name + ":" + e.message : "ok";
      });
      f.errors_engine_hash = simpleHash(msgs.join("|")).slice(0, 12);
      f.errors_engine_chrome_like = msgs.some(function (m) {
        return /Cannot read propert|is not a function|undefined/.test(m);
      });
      f.data_ok = true;
      enqueue(ctx, "B43_errors_engine", f, 39, "main");
    },
  });
  register("B44_speech_deep", {
    priority: 37,
    schedule: "dynamic",
    batch_id: "B44_speech_deep",
    layer: "deep",
    run: function (ctx) {
      var f = {
        speech_algo: "gr_speech_deep_v1",
        collected_at: Date.now(),
      };
      function packVoices(vs) {
        f.speech_voices_count = vs.length;
        f.speech_langs = [];
        var sample = [];
        var seen = {};
        vs.forEach(function (v) {
          var lang = v.lang || "";
          if (lang && !seen[lang]) {
            seen[lang] = 1;
            f.speech_langs.push(lang);
          }
          if (sample.length < 24) {
            sample.push({
              name: String(v.name || "").slice(0, 48),
              lang: lang,
              local: !!v.localService,
              default: !!v.default,
            });
          }
        });
        f.speech_voices_sample = sample;
        f.speech_voices_hash = simpleHash(
          vs.map(function (v) { return (v.name || "") + "|" + (v.lang || ""); }).join(";")
        ).slice(0, 12);
        f.data_ok = vs.length >= 0;
        enqueue(ctx, "B44_speech_deep", f, 37, "main");
      }
      try {
        if (!window.speechSynthesis) {
          f.speech_skip = "no_speechSynthesis";
          f.speech_voices_count = 0;
          f.data_ok = true;
          enqueue(ctx, "B44_speech_deep", f, 37, "main");
          return;
        }
        var vs = speechSynthesis.getVoices() || [];
        if (vs.length) {
          packVoices(vs);
          return;
        }
        // chrome loads async
        var done = false;
        speechSynthesis.onvoiceschanged = function () {
          if (done) return;
          done = true;
          packVoices(speechSynthesis.getVoices() || []);
        };
        setTimeout(function () {
          if (done) return;
          done = true;
          packVoices(speechSynthesis.getVoices() || []);
        }, 400);
      } catch (e) {
        f.speech_error = String((e && e.message) || e);
        f.data_ok = false;
        enqueue(ctx, "B44_speech_deep", f, 37, "main");
      }
    },
  });
  register("B45_display_hdr", {
    priority: 36,
    schedule: "dynamic",
    batch_id: "B45_display_hdr",
    layer: "deep",
    run: function (ctx) {
      var f = {
        display_algo: "gr_h11_display_hdr_v1",
        pohw_direction: "H11",
        collected_at: Date.now(),
      };
      try {
        var smH11 = readScreenMetrics();
        f.device_pixel_ratio = window.devicePixelRatio || null;
        f.screen_width = smH11.screen_width;
        f.screen_height = smH11.screen_height;
        f.screen_avail_width = smH11.screen_avail_width;
        f.screen_avail_height = smH11.screen_avail_height;
        f.screen_fp_protection_suspect = smH11.screen_fp_protection_suspect;
        f.screen_color_depth = smH11.color_depth;
        f.screen_pixel_depth = smH11.pixel_depth;
        f.inner_width = window.innerWidth;
        f.outer_width = window.outerWidth;
        try {
          f.orientation_type = screen.orientation && screen.orientation.type;
          f.orientation_angle = screen.orientation && screen.orientation.angle;
        } catch (eO) {}
        // matchMedia display capabilities
        var mqs = [
          "(color-gamut: srgb)",
          "(color-gamut: p3)",
          "(color-gamut: rec2020)",
          "(dynamic-range: high)",
          "(video-dynamic-range: high)",
          "(prefers-contrast: more)",
          "(prefers-reduced-transparency: reduce)",
          "(update: fast)",
          "(hover: hover)",
          "(pointer: fine)",
          "(any-pointer: coarse)",
        ];
        f.display_mq = {};
        mqs.forEach(function (q) {
          try {
            f.display_mq[q] = window.matchMedia(q).matches;
          } catch (e) {
            f.display_mq[q] = null;
          }
        });
        f.display_mq_hash = simpleHash(
          mqs.map(function (q) { return f.display_mq[q] ? "1" : "0"; }).join("")
        ).slice(0, 12);
        f.css_color_gamut = f.display_mq["(color-gamut: p3)"]
          ? "p3"
          : f.display_mq["(color-gamut: rec2020)"]
            ? "rec2020"
            : f.display_mq["(color-gamut: srgb)"]
              ? "srgb"
              : "unknown";
        f.hdr_likely = !!(f.display_mq["(dynamic-range: high)"] || f.display_mq["(video-dynamic-range: high)"]);
        f.data_ok = true;
      } catch (e) {
        f.display_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B45_display_hdr", f, 36, "main");
    },
  });

  C.__midMiscLoaded = true;
  C.__midFamilies = C.__midFamilies || {};
  C.__midFamilies.misc = true;
  // Full mid loaded if all three families present (compat).
  if (C.__midFamilies.core && C.__midFamilies.gpu && C.__midFamilies.misc) {
    C.__midLoaded = true;
  }
})(typeof window !== "undefined" ? window : globalThis);
