/* green-v5 registry.static.hard — B10/B7 wave2; auto-split; do not edit by hand */
(function (global) {
  "use strict";
  var C = global.GRCollectors;
  if (!C || !C.register) {
    try { console.warn("[gr] registry.static.hard: lite static not loaded"); } catch (e) {}
    return;
  }
  // Allow force re-register after product_version hot-swap (sticky B10 fix).
  // Also re-register when lite algo identity is missing/wrong (old closed-over run).
  var liteAlgoOk = false;
  try {
    var algoId = (C.__h && C.__h.cpu_loop_algo_id) || global.__GR_CPU_LOOP_ALGO__ || global.__GR_LITE_BUILD_ALGO__ || "";
    liteAlgoOk =
      algoId === "gr_cpu_curve_v2_multiworkload" ||
      algoId === "gr_cpu_curve_v3_multiround_median";
  } catch (eAlgo) {}
  if (C.get && C.get("B10_hw_curves") && C.get("B7_sandbox") && !global.__GR_FORCE_HARD_RELOAD__ && liteAlgoOk) {
    C.__hardLoaded = true;
    return;
  }
  if (global.__GR_FORCE_HARD_RELOAD__) {
    try { C.__hardLoaded = false; } catch (eForce) {}
  }
  if (C.__hardLoaded && !(C.get && C.get("B10_hw_curves"))) {
    C.__hardLoaded = false;
  }
  var register = C.register.bind(C);
  var h = C.__h || {};
  var midFields = h.midFields;
  var midEnqueue = h.midEnqueue;
  var enqueue = h.enqueue;
  var machineStableSignals = h.machineStableSignals;
  var deepMachineProbes = h.deepMachineProbes;
  var simpleHash = h.simpleHash;
  var simpleHash16 = h.simpleHash16;
  var probeWithFallbacks = h.probeWithFallbacks;
  var orderPathsForEngine = h.orderPathsForEngine;
  var enrichBatchFallbacksSync = h.enrichBatchFallbacksSync;
  var hardwareInventoryMultiPath = h.hardwareInventoryMultiPath;
  var hardwareInventoryAsyncFill = h.hardwareInventoryAsyncFill;
  var webglResidualMean = h.webglResidualMean;
  var webglResidualCurveV3e = h.webglResidualCurveV3e;
  var webglResidualMultiPath = h.webglResidualMultiPath;
  var yieldProbeGap = h.yieldProbeGap;
  var webrtcHostHashQuick = h.webrtcHostHashQuick;
  var osInstanceHashQuick = h.osInstanceHashQuick;
  var detectEngineFamily = h.detectEngineFamily;
  var isGeckoEngine = h.isGeckoEngine;
  var installTriggerPresentQuiet = h.installTriggerPresentQuiet;
  var readScreenMetrics = h.readScreenMetrics;
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
  if (typeof hwNoiseProbes !== "function" || typeof midFields !== "function") {
    try { console.warn("[gr] registry.static.hard: helpers missing"); } catch (e2) {}
    return;
  }
  try {
    var hardImpl0 = (typeof global.__GR_BUILD_IMPL__ !== "undefined" && global.__GR_BUILD_IMPL__) ||
      global.__GR_SERVER_PRODUCT_VERSION__ || global.__GR_PRODUCT_VERSION__ || "";
    if (hardImpl0) {
      global.__GR_FE_HARD_IMPL__ = String(hardImpl0);
      if (global.GRFeImpl && global.GRFeImpl.noteModule) global.GRFeImpl.noteModule("hard", hardImpl0);
      else { global.__GR_FE_IMPL__ = global.__GR_FE_IMPL__ || {}; global.__GR_FE_IMPL__.hard = String(hardImpl0); }
    }
  } catch (eHardImpl) {}

  register("B10_hw_curves", {
    // Align catalog: static high-priority hard anchors for commercial device_id
    priority: 91,
    schedule: "static",
    batch_id: "B10_hw_curves",
    layer: "hard",
    run: function (ctx) {
      // Seal v2 (prod) requires v3 multiround; lite stamps cpu_loop_algo_id as v3.
      // Accept v2 only as sticky-migration content gate (not for seal body).
      var EXPECTED = "gr_cpu_curve_v3_multiround_median";
      var EXPECTED_LEGACY = "gr_cpu_curve_v2_multiworkload";
      function liveAlgoId() {
        try {
          var C0 = global.GRCollectors;
          var h0 = (C0 && C0.__h) || {};
          return String(
            h0.cpu_loop_algo_id ||
              global.__GR_CPU_LOOP_ALGO__ ||
              global.__GR_LITE_BUILD_ALGO__ ||
              ""
          );
        } catch (eA) {
          return "";
        }
      }
      function algoOkForStamp(a) {
        return a === EXPECTED || a === EXPECTED_LEGACY || String(a).indexOf("v3_multiround") >= 0;
      }
      // P2 content gate: sticky old lite must reload before B10 runs.
      if (!algoOkForStamp(liveAlgoId()) && !global.__GR_B10_CONTENT_GATE_TRIED__) {
        global.__GR_B10_CONTENT_GATE_TRIED__ = 1;
        var svGate = String(
          global.__GR_SERVER_PRODUCT_VERSION__ ||
            global.__GR_PRODUCT_VERSION__ ||
            (global.__GR_BOOT__ &&
              (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
            ""
        );
        var reloadFn =
          typeof global.ensurePacksMatchServerVersion === "function"
            ? global.ensurePacksMatchServerVersion
            : typeof global.reloadStaticCollectors === "function"
              ? global.reloadStaticCollectors
              : null;
        if (reloadFn && svGate) {
          return Promise.resolve(reloadFn(svGate))
            .catch(function () {
              return false;
            })
            .then(function (ok) {
              var algoNow = liveAlgoId();
              var contentOk = algoOkForStamp(algoNow);
              try {
                if (global.GROps && GROps.report) {
                  // Severity follows content identity, not reload Promise alone
                  // (reload may return false while live algo is already v3).
                  GROps.report(
                    "b10_content_gate",
                    "probe",
                    {
                      ok: !!(ok && contentOk),
                      algo: algoNow,
                      expected: EXPECTED,
                      expected_legacy: EXPECTED_LEGACY,
                      v: svGate,
                    },
                    contentOk ? "info" : "warn"
                  );
                }
              } catch (eRep) {}
              try {
                var b10n =
                  global.GRCollectors &&
                  global.GRCollectors.get &&
                  global.GRCollectors.get("B10_hw_curves");
                if (b10n && typeof b10n.run === "function" && contentOk) {
                  return b10n.run(ctx);
                }
              } catch (eRe) {}
              return runB10Body(ctx);
            });
        }
      }
      return runB10Body(ctx);

      function runB10Body(ctx) {
      var f = midFields("hw_curves");
      try {
        f.perf_now = typeof performance !== "undefined" ? performance.now() : null;
        f.time_origin = typeof performance !== "undefined" ? performance.timeOrigin : null;
      } catch (e) {}
      // Pack stamp: prefer content-proven packs when algo is known-good.
      // Seal gate binds fe_impl_version to **server product epoch** — never leave empty
      // when lite is already on v3 while hard EXPECTED lagged (caused 422 fe_impl_missing).
      try {
        var g = typeof global !== "undefined" ? global : window;
        var algoLive = liveAlgoId();
        var productFe = String(
          (g && g.__GR_SERVER_PRODUCT_VERSION__) ||
            (g && g.__GR_PRODUCT_VERSION__) ||
            (g && g.__GR_BOOT__ && (g.__GR_BOOT__.product_version || g.__GR_BOOT__.version)) ||
            ""
        );
        var packsOnly = "";
        if (algoOkForStamp(algoLive)) {
          packsOnly = String(
            (g && g.__GR_FE_PACKS_VERSION__) ||
              (g && g.__GR_FE_HARD_IMPL__) ||
              (g && g.__GR_FE_LITE_IMPL__) ||
              (g && g.__GR_BUILD_IMPL__) ||
              productFe ||
              ""
          );
          if (packsOnly && g) {
            try {
              if (!g.__GR_FE_PACKS_VERSION__) g.__GR_FE_PACKS_VERSION__ = packsOnly;
              if (g.GRFeImpl && g.GRFeImpl.markContentProven) {
                g.GRFeImpl.markContentProven(packsOnly);
              }
            } catch (eMk) {}
          }
        }
        f.fe_packs_version = packsOnly || productFe;
        f.fe_code_version = String((g && g.__GR_FE_CODE_VERSION__) || packsOnly || productFe || "");
        f.fe_asset_version = packsOnly || f.fe_code_version || productFe || "";
        // Seal allowlist = product epoch (6.0.7); content/build remain side diagnostics.
        f.fe_impl_version = String(
          productFe ||
            packsOnly ||
            (g && g.__GR_FE_IMPL_VERSION__) ||
            (g && g.GRFeImpl && g.GRFeImpl.snapshot && g.GRFeImpl.snapshot().fe_impl_version) ||
            ""
        );
        f.cpu_loop_algo_build = String(
          (g && g.__GR_CPU_LOOP_ALGO__) ||
            (g && g.GRCollectors && g.GRCollectors.__h && g.GRCollectors.__h.cpu_loop_algo_id) ||
            ""
        );
        f.product_version_fe = productFe;
        f.server_product_version = productFe;
        f.fe_lite_impl = String((g && g.__GR_FE_LITE_IMPL__) || "");
        f.fe_hard_impl = String((g && g.__GR_FE_HARD_IMPL__) || "");
        f.fe_build_impl = String((g && g.__GR_BUILD_IMPL__) || "");
        f.fe_loader_impl = String(
          (g && g.__GR_FE_LOADER_IMPL__) ||
            (g && g.__GR_FE_IMPL__ && g.__GR_FE_IMPL__.loader) ||
            ""
        );
      } catch (eFeV) {}
      // Do NOT start inventory before residual: inventory also opens WebGL + OfflineAudio
      // and concurrent same-hardware thrash freezes weak tabs (v5.8.137 pressure fix).
      // Resolve hwNoiseProbes at RUN time from live __h (not closed-over at hard register).
      // Sticky hard early-return: accept v2 or v3 multiround cpu algo (lab K/V stability).
      function liveHwNoiseProbes() {
        try {
          var C0 = typeof global !== "undefined" ? global.GRCollectors : null;
          var h0 = (C0 && C0.__h) || {};
          if (typeof h0.hwNoiseProbes === "function") return h0.hwNoiseProbes;
          if (C0 && typeof C0.hwNoiseProbes === "function") return C0.hwNoiseProbes;
        } catch (eLive) {}
        return hwNoiseProbes;
      }
      // Inventory only after noise residual (serial same-hardware access).
      var invP = null;
      function startInventoryAfterResidual() {
        if (invP) return invP;
        invP = Promise.resolve()
          .then(function () {
            return yieldProbeGap();
          })
          .then(function () {
            return hardwareInventoryAsyncFill(hardwareInventoryMultiPath());
          })
          .catch(function () {
            return {};
          });
        return invP;
      }
      var noiseP = liveHwNoiseProbes()();
      var noiseBound = new Promise(function (resolve) {
        var settled = false;
        var to = setTimeout(function () {
          if (settled) return;
          settled = true;
          resolve({ __timeout: true });
        }, 18000);
        Promise.resolve(noiseP).then(
          function (n) {
            if (settled) return;
            settled = true;
            clearTimeout(to);
            resolve(n || {});
          },
          function () {
            if (settled) return;
            settled = true;
            clearTimeout(to);
            resolve({});
          }
        );
      });
      return noiseBound.then(function (noise) {
      if (noise && noise.__timeout) {
        try {
          f.b10_noise_timeout = true;
        } catch (eNt) {}
        noise = {};
      }
        if (noise) {
          Object.keys(noise).forEach(function (k) {
            if (k === "_audio_p") return;
            f[k] = noise[k];
          });
        }
        // Compact keys for server commercial digest + soft cosine
        if (noise && noise.hw_noise_curves) {
          f.hw_curve_audio = noise.hw_noise_curves.audio || null;
          f.hw_curve_canvas = noise.hw_noise_curves.canvas || null;
          // cp silicon = wall-clock CPU timing curve (NOT deterministic jit lowbits)
          f.hw_curve_cpu = noise.hw_noise_curves.cpu || null;
          f.cpu_timing_curve = noise.hw_noise_curves.cpu || f.cpu_timing_curve || null;
          f.hw_curve_webgl = noise.hw_noise_curves.webgl || null;
          // Seed-delta preferred commercial au material (class-floor breaker)
          if (noise.hw_noise_curves.audio_delta && noise.hw_noise_curves.audio_delta.length) {
            f.audio_seed_delta_curve = noise.hw_noise_curves.audio_delta;
            f.audio_noise_delta = noise.hw_noise_curves.audio_delta;
            f.hw_curve_audio_delta = noise.hw_noise_curves.audio_delta;
          }
          // jit is diagnostic-only (IEEE-754 identical across engines); never overwrite cpu
          if (noise.hw_noise_curves.jit && noise.hw_noise_curves.jit.length) {
            f.jit_lowbits_curve = noise.hw_noise_curves.jit;
          }
          if (noise.hw_noise_curves.timing && noise.hw_noise_curves.timing.length) {
            f.timing_jitter_curve = noise.hw_noise_curves.timing;
          }
        }
        if (noise && noise.audio_seed_delta_curve && noise.audio_seed_delta_curve.length) {
          f.audio_seed_delta_curve = noise.audio_seed_delta_curve;
          f.audio_noise_delta = noise.audio_seed_delta_curve;
        }
        if (noise) {
          if (noise.jit_lowbits_curve) f.jit_lowbits_curve = noise.jit_lowbits_curve;
          if (noise.jit_lowbits_algo) f.jit_lowbits_algo = noise.jit_lowbits_algo;
          if (noise.timing_jitter_curve) f.timing_jitter_curve = noise.timing_jitter_curve;
          if (noise.raf_interval_curve) f.raf_interval_curve = noise.raf_interval_curve;
          if (noise.raf_hz_est != null) f.raf_hz_est = noise.raf_hz_est;
          if (noise.raf_algo) f.raf_algo = noise.raf_algo;
          if (noise.canvas_noise_patterns != null) f.canvas_noise_patterns = noise.canvas_noise_patterns;
          if (noise.canvas_noise_algo) f.canvas_noise_algo = noise.canvas_noise_algo;
          // Multiround materials for commercial cp/tz analysis (hw_probe_analysis_v1)
          if (noise.cpu_timing_rounds && noise.cpu_timing_rounds.length) {
            f.cpu_timing_rounds = noise.cpu_timing_rounds;
            f.hw_curve_cpu_rounds = noise.hw_curve_cpu_rounds || noise.cpu_timing_rounds;
          }
          if (noise.cpu_loop_algo) f.cpu_loop_algo = noise.cpu_loop_algo;
          if (noise.cpu_loop_rounds != null) f.cpu_loop_rounds = noise.cpu_loop_rounds;
          if (noise.timing_jitter_rounds && noise.timing_jitter_rounds.length) {
            f.timing_jitter_rounds = noise.timing_jitter_rounds;
          }
          if (noise.raf_interval_rounds && noise.raf_interval_rounds.length) {
            f.raf_interval_rounds = noise.raf_interval_rounds;
          }
          if (noise.raf_rounds != null) f.raf_rounds = noise.raf_rounds;
        }
        // Explicit residual metadata (must not be lost if nested under noise)
        if (noise) {
          if (noise.residual_algo) f.residual_algo = noise.residual_algo;
          if (noise.residual_mean != null && !isNaN(Number(noise.residual_mean))) {
            f.residual_mean = Number(noise.residual_mean);
            f.residual_available = true;
          }
          if (noise.residual_std != null && !isNaN(Number(noise.residual_std))) {
            f.residual_std = Number(noise.residual_std);
          }
          if (noise.residual_hist) f.residual_hist = noise.residual_hist;
          if (noise.unit_surface_id) f.unit_surface_id = noise.unit_surface_id;
          if (noise.unit_surface_algo) f.unit_surface_algo = noise.unit_surface_algo;
          if (noise.unit_multiround_stable != null) {
            f.unit_multiround_stable = noise.unit_multiround_stable;
          }
          if (noise.audio_noise_algo) f.audio_noise_algo = noise.audio_noise_algo;
          if (noise.audio_noise_seeds != null) f.audio_noise_seeds = noise.audio_noise_seeds;
          if (noise.audio_probe_path) f.audio_probe_path = noise.audio_probe_path;
          if (noise.webgl_probe_path) f.webgl_probe_path = noise.webgl_probe_path;
          if (noise.canvas_probe_path) f.canvas_probe_path = noise.canvas_probe_path;
        }
        // Derive residual_mean from webgl curve when still missing
        if (
          (f.residual_mean == null || isNaN(Number(f.residual_mean))) &&
          f.hw_curve_webgl &&
          f.hw_curve_webgl.length
        ) {
          try {
            var si;
            var ss = 0;
            var sn = 0;
            for (si = 0; si < f.hw_curve_webgl.length; si++) {
              var sv = Number(f.hw_curve_webgl[si]);
              if (!isNaN(sv) && isFinite(sv)) {
                ss += sv;
                sn++;
              }
            }
            if (sn > 0) {
              f.residual_mean = Math.round((ss / sn) * 1e12) / 1e12;
              f.residual_available = true;
            }
          } catch (eMean) {}
        }
        try {
          if (f.residual_mean != null) global.__GR_MAIN_RESIDUAL_MEAN__ = f.residual_mean;
        } catch (eMainRm) {}
        if (!f.residual_algo && f.hw_curve_webgl && f.hw_curve_webgl.length) {
          f.residual_algo = "gr_webgl_residual_std_v3f";
        }
        // residual_ok: explicit boolean for server merge / digest honesty
        f.residual_ok =
          (f.residual_std != null && Number(f.residual_std) > 0) ||
          (f.residual_mean != null && f.residual_available === true) ||
          !!(f.hw_curve_webgl && f.hw_curve_webgl.length >= 8);
        f.residual_available = !!f.residual_ok || !!f.residual_available;
        // OS instance + ICE host hash (real-path digest host separators)
        var osi = {};
        try {
          osi = osInstanceHashQuick() || {};
        } catch (eOsi) {
          osi = {};
        }
        if (osi.os_instance_hash) {
          f.os_instance_hash = osi.os_instance_hash;
          f.os_instance_source = osi.os_instance_source || "proxy_composite";
        }
        // Engine-aware ICE budget (WebKit longer); do not hardcode 2s for all engines.
        f.engine_family = f.engine_family || detectEngineFamily();
        f.probe_profile = f.probe_profile || probeProfileForEngine(f.engine_family);
        // --- Early silicon land (critical) ---
        // Residual/noise is GPU-class work. Do NOT hold pack_loader gpu lock waiting for
        // media inventory / WebRTC ICE (can be multi-second or hang in lab CDP).
        // 1) enqueue residual-ready B10 immediately
        // 2) schedule eager B10x + B47/B46/B18 so they start once lock releases
        // 3) fire-and-forget inv+webrtc enrichment as a second B10 upload
        f.b10_meta = {
          residual_algo: f.residual_algo || null,
          residual_mean: f.residual_mean != null ? f.residual_mean : null,
          residual_std: f.residual_std != null ? f.residual_std : null,
          residual_ok: !!f.residual_ok,
          engine_family: f.engine_family || null,
          probe_profile: f.probe_profile || null,
          residual_probe_profile: f.residual_probe_profile || null,
          has_webgl_curve: !!(f.hw_curve_webgl && f.hw_curve_webgl.length),
          has_audio_curve: !!(f.hw_curve_audio && f.hw_curve_audio.length),
          audio_noise_algo: f.audio_noise_algo || null,
          audio_noise_seeds: f.audio_noise_seeds != null ? f.audio_noise_seeds : null,
          audio_probe_path: f.audio_probe_path || null,
          webgl_probe_path: f.webgl_probe_path || null,
          canvas_probe_path: f.canvas_probe_path || null,
          has_os_instance: !!f.os_instance_hash,
          os_instance_source: f.os_instance_source || null,
          unit_surface_id: f.unit_surface_id || null,
          b10_phase: "residual_early",
        };
        f.probe_paths = (f.probe_paths || []).concat([
          { name: "audio_curve", ok: !!(f.hw_curve_audio && f.hw_curve_audio.length), detail: f.audio_probe_path || null },
          { name: "webgl_curve", ok: !!(f.hw_curve_webgl && f.hw_curve_webgl.length), detail: f.webgl_probe_path || null },
          { name: "canvas_curve", ok: !!(f.hw_curve_canvas && f.hw_curve_canvas.length), detail: f.canvas_probe_path || null },
          { name: "cpu_curve", ok: !!(f.hw_curve_cpu && f.hw_curve_cpu.length), detail: f.cpu_loop_algo || null },
        ]);
        var earlyEnq = midEnqueue(ctx, "B10_hw_curves", f, 91);
        setTimeout(function () {
          try {
            if (
              global.GRCollectors &&
              typeof global.GRCollectors.scheduleEagerB10xChain === "function"
            ) {
              global.GRCollectors.scheduleEagerB10xChain(ctx);
            } else {
              scheduleEagerB10xChain(ctx);
            }
          } catch (eB10x) {}
        }, 40);
        // Enrichment (inventory + ICE) with hard timeout — never block silicon chain.
        function withTimeout(p, ms, label) {
          return Promise.race([
            Promise.resolve(p).catch(function () {
              return null;
            }),
            new Promise(function (resolve) {
              setTimeout(function () {
                resolve({ __timeout: true, __label: label || "b10_enrich" });
              }, ms);
            }),
          ]);
        }
        withTimeout(startInventoryAfterResidual(), 3500, "inv")
          .then(function (inv) {
            if (inv && !inv.__timeout) {
              Object.keys(inv).forEach(function (k) {
                if (k === "probe_paths" || k.indexOf("_") === 0) return;
                if (inv[k] != null && inv[k] !== "" && f[k] == null) f[k] = inv[k];
              });
              if (inv.probe_paths && inv.probe_paths.length) {
                f.probe_paths = (inv.probe_paths || []).concat(f.probe_paths || []);
              }
            } else if (inv && inv.__timeout) {
              f.b10_inv_timeout = true;
            }
            return withTimeout(webrtcHostHashQuick(), 4500, "webrtc");
          })
          .then(function (rtc) {
            if (rtc && !rtc.__timeout) {
              Object.keys(rtc).forEach(function (k) {
                if (rtc[k] != null && rtc[k] !== "") f[k] = rtc[k];
              });
              if (f.webrtc_host_ip_hash == null && rtc.webrtc_host_count === 0) {
                f.webrtc_host_count = 0;
              }
              if (rtc.webrtc_host_ips && rtc.webrtc_host_ips.length) {
                f.webrtc_host_ip_hash_v2 = simpleHash16(
                  rtc.webrtc_host_ips.slice().sort().join("|")
                );
              }
            } else if (rtc && rtc.__timeout) {
              f.b10_webrtc_timeout = true;
              f.webrtc_probe_failed = f.webrtc_probe_failed || "timeout";
            }
            f.b10_meta = Object.assign({}, f.b10_meta || {}, {
              webrtc_method: f.webrtc_method || null,
              has_webrtc: !!f.webrtc_host_ip_hash,
              media_device_count: f.media_device_count != null ? f.media_device_count : null,
              display_count: f.display_count != null ? f.display_count : null,
              probe_path_ok_n: (f.probe_paths || []).filter(function (p) {
                return p && p.ok;
              }).length,
              b10_phase: "enriched",
            });
            // Second upload with force so enrichment lands (force_after_halt via B10 priority)
            try {
              midEnqueue(ctx, "B10_hw_curves", f, 90);
            } catch (eEnq2) {}
          })
          .catch(function () {});
        return earlyEnq;
      });
      } // runB10Body
    },
  });
  register("B7_sandbox", {
    priority: 45,
    schedule: "static",
    batch_id: "B7_sandbox",
    layer: "deep",
    run: function (ctx) {
      if (!global.GRSandbox) return null;
      var plan =
        (ctx && ctx.sandbox_plan) ||
        (global.__GR_SANDBOX_PLAN__ != null ? global.__GR_SANDBOX_PLAN__ : null);
      // Nest kinds are identity + lite silicon (nest_* fields only — never residual_mean).
      // Stagger: maxConcurrent=1 so iframe/worker never dual-fire heavy GPU with each other.
      // B7 itself is HW-class via kickAll withHardwareLock vs B10/B10x.
      var plan2 = plan
        ? Object.assign({}, plan, { max_concurrent: 1 })
        : { max_concurrent: 1 };
      return global.GRSandbox.runTree(ctx, {
        maxConcurrent: 1,
        short_visit: true,
        sandbox_plan: plan2,
      });
    },
  });


  // --- B10x silicon packs (inline; do not require registry.b10x.js) ---
  function runHardB10x(ctx, packId, profile, prio) {
    var f = midFields(packId || "b10x");
    f.engine_family = typeof detectEngineFamily === "function" ? detectEngineFamily() : "unknown";
    f.probe_profile = typeof probeProfileForEngine === "function" ? probeProfileForEngine(f.engine_family) : "probe_default_v1";
    f.b10x_pack = packId;
    f.b10x_profile = profile || "silicon_noderiv";
    f.b10x_inline = true;
    f.b10x_phase = "start";
    // Immediate start marker — source "start" so alreadySent(main) does NOT block
    // the terminal done upload if multipath dies mid-flight.
    try {
      var Q = (ctx && ctx.queue) || global.GRUploadQueue;
      var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
      if (Q && Q.enqueue && sid) {
        Q.enqueue({
          session_id: sid,
          batch_id: packId,
          source: "start",
          inject_path: (ctx && ctx.inject_path) || "",
          priority: prio || 90,
          force: true,
          force_after_halt: true,
          payload: { fields: Object.assign({}, f, { b10x_ok: false, b10x_err: "started", b10x_phase: "start" }), sandbox_kind: "main" }
        });
      }
    } catch (e0) {}
    if (typeof webglResidualMultiPath !== "function") {
      f.b10x_ok = false;
      f.b10x_err = "no_multipath";
      f.b10x_phase = "done";
      f.residual_paths_n = 0;
      return midEnqueue(ctx, packId, f, prio || 90);
    }
    var mpP = webglResidualMultiPath({ profile: profile || "silicon_noderiv" });
    var timed = new Promise(function (resolve) {
      var done=false;
      var to=setTimeout(function(){ if(!done){ done=true; resolve({__timeout:true}); } }, 20000);
      Promise.resolve(mpP).then(function(v){ if(!done){ done=true; clearTimeout(to); resolve(v); } },
        function(e){ if(!done){ done=true; clearTimeout(to); resolve({__error:String(e&&e.message||e)}); } });
    });
    return timed
      .then(function (mp) {
        if (!mp || mp.__timeout || mp.__error) {
          f.b10x_ok = false;
          f.b10x_err = (mp && mp.__timeout) ? "multipath_timeout" : ((mp && mp.__error) || "multipath_empty");
          f.b10x_timeout = !!(mp && mp.__timeout);
          f.b10x_phase = "done";
          f.residual_paths_n = f.residual_paths_n != null ? f.residual_paths_n : 0;
          return midEnqueue(ctx, packId, f, prio || 90);
        }
        f.b10x_ok = !!(mp.curve && mp.curve.length >= 8);
        f.residual_paths = mp.residual_paths || [];
        f.residual_select = mp.residual_select || null;
        f.residual_probe_engine = mp.residual_probe_engine || f.engine_family;
        f.residual_probe_profile = mp.residual_probe_profile || f.probe_profile;
        f.multipath_profile = mp.multipath_profile || profile;
        f.webgl_probe_path = "b10x_hard_" + (profile || "silicon");
        if (mp.curve && mp.curve.length) {
          f.hw_curve_webgl = mp.curve;
          f.hw_noise_curves = f.hw_noise_curves || {};
          f.hw_noise_curves.webgl = mp.curve;
        }
        if (mp.residual_mean != null) {
          f.residual_mean = mp.residual_mean;
          f.residual_available = true;
          try { global.__GR_MAIN_RESIDUAL_MEAN__ = mp.residual_mean; } catch (eRm) {}
        }
        if (mp.residual_std != null) f.residual_std = mp.residual_std;
        if (mp.residual_algo) f.residual_algo = mp.residual_algo;
        f.residual_ok =
          (f.residual_std != null && Number(f.residual_std) > 0) ||
          (f.residual_mean != null && f.residual_available === true) ||
          !!(f.hw_curve_webgl && f.hw_curve_webgl.length >= 8);
        f.residual_paths_n = (f.residual_paths || []).length;
        try {
          var seedFn = (global.GRCollectors && global.GRCollectors.__h && global.GRCollectors.__h.attachSeededReplayFields) || null;
          if (typeof seedFn === "function") seedFn(f, ctx, profile);
        } catch (eSeed) { f.seed_replay_err = String(eSeed && eSeed.message || eSeed); }
        try {
          var relFn = (global.GRCollectors && global.GRCollectors.__h && global.GRCollectors.__h.releaseWebglProbeContexts) || null;
          if (typeof relFn === "function") relFn();
        } catch (eRel) {}
        if (!f.b10x_ok && !f.b10x_err) {
          f.b10x_err = f.residual_paths_n > 0 ? "weak_curve" : (profile === "silicon_ulp" ? "ulp_no_entropy" : "no_curve");
        }
        f.b10x_phase = "done";
        f.b10x_meta = {
          pack_id: packId,
          profile: profile,
          n_paths: f.residual_paths_n,
          chosen: f.residual_select && f.residual_select.chosen_path_id,
          mean: f.residual_mean,
          std: f.residual_std,
          hard_inline: true,
          seed_residual_digest: f.seed_residual_digest || null,
        };
        return midEnqueue(ctx, packId, f, prio || 90);
      })
      .catch(function (e) {
        f.b10x_ok = false;
        f.b10x_err = String(e && e.message ? e.message : e);
        f.b10x_phase = "done";
        f.residual_paths_n = f.residual_paths_n != null ? f.residual_paths_n : 0;
        return midEnqueue(ctx, packId, f, prio || 90);
      });
  }
  [
    ["B10x_silicon_ulp", "silicon_ulp", 90],
    ["B10x_silicon_noderiv", "silicon_noderiv", 89],
    ["B10x_silicon_rint", "silicon_rint", 89],
    ["B10x_silicon_deep", "silicon_deep", 91],
    ["B10x_webkit_gl_noise", "webkit_deep", 88],
    ["B10x_webkit_wave2_warm4", "webkit_wave2", 86],
    ["B10x_angle_crosscheck", "angle_cross", 80],
    ["B10x_softgl_hedge", "softgl", 84],
    ["B10x_legacy_webgl1", "legacy_webgl1", 83],
    ["B10x_unknown_kernel", "unknown", 87]
  ].forEach(function (row) {
    var id = row[0], profile = row[1], prio = row[2];
    if (C.get && C.get(id) && C.get(id).run) return;
    register(id, {
      priority: prio,
      schedule: "dynamic",
      batch_id: id,
      layer: "hard",
      pack_lane: "deepen",
      _hard_inline: true,
      run: function (ctx) {
        var PL = global.GRPackLoader;
        var exec = function () {
          return runHardB10x(ctx, id, profile, prio);
        };
        if (PL && typeof PL.withHardwareLock === "function") {
          return PL.withHardwareLock("b10x_hard:" + id, exec);
        }
        return exec();
      }
    });
  });
  // Eager: ulp→rint→noderiv→deep (iss/62 required trio first; deep last).
  // Prefer fuller scheduleEagerSecondaryInfra / scheduleEagerB10xChain from static if present.
  if (typeof C.scheduleEagerSecondaryInfra === "function" && C.scheduleEagerB10xChain && C.scheduleEagerB10xChain.__ssot) {
    // static already installed fuller chain
  } else {
  C.scheduleEagerB10xChain = function (ctx) {
    // P0: engine-aware B10x order (fallback if method matrix / full static chain missing)
    var silicon = ["B10x_silicon_noderiv", "B10x_silicon_rint", "B10x_silicon_ulp", "B10x_silicon_deep"];
    try {
      if (global.GRProbeMethodMatrix && GRProbeMethodMatrix.engineB10xOrder) {
        var o = GRProbeMethodMatrix.engineB10xOrder();
        if (o && o.length) silicon = o;
      }
    } catch (eSi) {}
    try { global.__GR_EAGER_B10X__ = { at: Date.now(), packs: silicon.slice() }; } catch (eM) {}
    var PL = global.GRPackLoader;
    var Q = (ctx && ctx.queue) || global.GRUploadQueue;
    var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
    function kickSecondary(includeGpu) {
      if (typeof C.scheduleEagerSecondaryInfra === "function") {
        return C.scheduleEagerSecondaryInfra(ctx, { skipParallel: !includeGpu ? false : true, includeGpu: !!includeGpu });
      }
      var ids = includeGpu ? ["B18_webgpu"] : ["B47_sab_clock", "B46_audio_deep"];
      var ensure = global.ensureMidModules;
      var start = function () {
        var jobs = ids.map(function (id) {
          return Promise.resolve().then(function () {
            var pack = C.get && C.get(id);
            if (!pack || typeof pack.run !== "function") return null;
            try {
              if (Q && Q.alreadySent && sid && Q.alreadySent({ session_id: sid, batch_id: id, source: "main" })) {
                if (id === "B47_sab_clock" || id === "B18_webgpu") {
                  try { Q.clearSentKeys && Q.clearSentKeys([{ session_id: sid, batch_id: id, source: "main" }]); } catch (eC) {}
                } else return null;
              }
            } catch (eS) {}
            return pack.run(Object.assign({}, ctx || {}, { session_id: sid, queue: Q }));
          }).catch(function () { return null; });
        });
        try { global.__GR_EAGER_SECONDARY__ = { at: Date.now(), packs: ids.slice(), hard_tail: true }; } catch (eM2) {}
        return Promise.all(jobs);
      };
      if (typeof ensure === "function") {
        return Promise.resolve(ensure({ packs: ids.map(function (id) { return { pack_id: id }; }) }))
          .catch(function () { return null; })
          .then(start);
      }
      return start();
    }
    try { kickSecondary(false); } catch (eSec) {}
    // Serial via kickAll one-by-one so deep is not starved by parallel gpu queue.
    var chain = Promise.resolve();
    silicon.forEach(function (id) {
      chain = chain.then(function () {
        var pack = C.get && C.get(id);
        if (!pack || typeof pack.run !== "function") return null;
        try {
          if (Q && Q.alreadySent && sid && Q.alreadySent({ session_id: sid, batch_id: id, source: "main" })) {
            if (id !== "B10x_silicon_deep") return null;
            try { Q.clearSentKeys && Q.clearSentKeys([{ session_id: sid, batch_id: id, source: "main" }]); } catch (eC) {}
          }
        } catch (eSkip) {}
        var runCtx = Object.assign({}, ctx || {}, { session_id: sid, queue: Q });
        var item = {
          id: id, pack_id: id, batch_id: id, priority: pack.priority || 90,
          schedule: "dynamic", layer: "hard",
          run: function (c) { return pack.run(c || runCtx); }
        };
        if (PL && typeof PL.kickAll === "function") return PL.kickAll([item], runCtx);
        return Promise.resolve().then(function () { return pack.run(runCtx); }).catch(function () { return null; });
      });
    });
    return chain
      .then(function () { return kickSecondary(true); })
      .then(function () { return kickSecondary(false); }) // B47 final guarantee
      .catch(function () { return kickSecondary(false); });
  };
  }
  // Hard-inline B47 skip if mid not yet loaded
  if (!(C.get && C.get("B47_sab_clock"))) {
    register("B47_sab_clock", {
      priority: 62, schedule: "dynamic", batch_id: "B47_sab_clock", layer: "mid",
      run: function (ctx) {
        var f = {
          sab_clock_algo: "gr_sab_clock_v1", pohw_direction: "A3", collected_at: Date.now(),
          cross_origin_isolated: typeof crossOriginIsolated !== "undefined" ? !!crossOriginIsolated : false,
          has_shared_array_buffer: typeof SharedArrayBuffer !== "undefined",
          has_atomics: typeof Atomics !== "undefined", b47_inline: true
        };
        if (!f.cross_origin_isolated) {
          f.sab_clock_skip = "need_coop_coep_cross_origin_isolated"; f.sab_clock_ok = false; f.data_ok = true;
        } else if (!f.has_shared_array_buffer || !f.has_atomics) {
          f.sab_clock_skip = "no_sab_or_atomics"; f.sab_clock_ok = false; f.data_ok = true;
        } else {
          f.sab_clock_skip = "deferred_full_calibrate"; f.sab_clock_ok = false; f.data_ok = true;
        }
        try {
          var Q = (ctx && ctx.queue) || global.GRUploadQueue;
          var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
          if (Q && Q.enqueue && sid) {
            Q.enqueue({ session_id: sid, batch_id: "B47_sab_clock", source: "main", priority: 62, force: true, force_after_halt: true, allow_during_stop: true, payload: { fields: f, sandbox_kind: "main" } });
          } else if (typeof midEnqueue === "function") {
            midEnqueue(ctx, "B47_sab_clock", f, 62);
          }
        } catch (eE) {}
      }
    });
  }

  C.__hardLoaded = true;
})(typeof window !== "undefined" ? window : globalThis);
