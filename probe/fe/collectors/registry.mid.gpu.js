/* green-v5 registry.mid.gpu — auto-split from registry.js; do not edit by hand */
(function (global) {
  "use strict";
  var C = global.GRCollectors;
  if (!C || !C.register) {
    try { console.warn("[gr] registry.mid.gpu: static registry not loaded"); } catch (e) {}
    return;
  }
  if (C.__midGpuLoaded || (C.get && C.get("B22_gpu_timer"))) {
    C.__midGpuLoaded = true;
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
    try { console.warn("[gr] registry.mid.gpu: helpers missing from static"); } catch (e2) {}
    return;
  }

  register("B17_hw_physical", {
    priority: 58,
    schedule: "dynamic",
    batch_id: "B17_hw_physical",
    layer: "hard",
    run: function (ctx) {
      var f = { hw_physical_algo: "gr_hw_physical_v3_multipath", collected_at: Date.now() };
      // Multi-path inventory (display/net/media/battery/gamepad/gl params)
      return hardwareInventoryAsyncFill(hardwareInventoryMultiPath())
        .catch(function () {
          return {};
        })
        .then(function (inv) {
          if (inv) {
            Object.keys(inv).forEach(function (k) {
              if (inv[k] != null && inv[k] !== "") f[k] = inv[k];
            });
          }
          try {
            var c = document.createElement("canvas");
            c.width = 64;
            c.height = 64;
            var gl =
              c.getContext("webgl2") ||
              c.getContext("webgl") ||
              c.getContext("experimental-webgl");
            if (gl) {
              f.gl_max_texture_size = gl.getParameter(gl.MAX_TEXTURE_SIZE);
              f.gl_max_renderbuffer = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE);
              f.gl_max_vertex_attribs = gl.getParameter(gl.MAX_VERTEX_ATTRIBS);
              f.gl_max_texture_image_units = gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS);
              f.gl_max_varying_vectors = gl.getParameter(gl.MAX_VARYING_VECTORS);
              f.gl_max_cube_map = gl.getParameter(gl.MAX_CUBE_MAP_TEXTURE_SIZE);
              var stages = [gl.VERTEX_SHADER, gl.FRAGMENT_SHADER];
              var types = [
                gl.LOW_FLOAT,
                gl.MEDIUM_FLOAT,
                gl.HIGH_FLOAT,
                gl.LOW_INT,
                gl.MEDIUM_INT,
                gl.HIGH_INT,
              ];
              var matrix = [];
              stages.forEach(function (st) {
                types.forEach(function (tp) {
                  try {
                    var p = gl.getShaderPrecisionFormat(st, tp);
                    matrix.push(p ? [p.rangeMin, p.rangeMax, p.precision] : null);
                  } catch (e) {
                    matrix.push(null);
                  }
                });
              });
              f.gl_precision_matrix = matrix;
              f.timer_query_available = !!(
                gl.getExtension("EXT_disjoint_timer_query") ||
                gl.getExtension("EXT_disjoint_timer_query_webgl2")
              );
              // H02 lite: small readPixels bandwidth staircase (64² and 128² wall ms)
              try {
                var sizes = [64, 128];
                var bw = [];
                sizes.forEach(function (sz) {
                  var c3 = document.createElement("canvas");
                  c3.width = sz;
                  c3.height = sz;
                  var g3 =
                    c3.getContext("webgl2", { preserveDrawingBuffer: true }) ||
                    c3.getContext("webgl", { preserveDrawingBuffer: true });
                  if (!g3) return;
                  g3.clearColor(0.2, 0.3, 0.4, 1);
                  g3.clear(g3.COLOR_BUFFER_BIT);
                  var pix = new Uint8Array(sz * sz * 4);
                  var t0 = performance.now();
                  g3.readPixels(0, 0, sz, sz, g3.RGBA, g3.UNSIGNED_BYTE, pix);
                  var ms = performance.now() - t0;
                  bw.push({
                    size: sz,
                    readback_ms: Math.round(ms * 1000) / 1000,
                    bytes: pix.length,
                  });
                });
                f.gl_bandwidth_lite = bw;
              } catch (eBw) {}
            }
          } catch (e) {}
          // CPU micro timing curve (H07 lite) — 24 samples for better digest stability
          try {
            var times = [];
            for (var k = 0; k < 24; k++) {
              var t0 = performance.now();
              var acc = 0;
              for (var n = 0; n < 4000; n++) acc += Math.sin(n * 0.019) * Math.cos(n * 0.011);
              times.push(performance.now() - t0);
              if (acc === Infinity) times.push(0);
            }
            f.cpu_timing_curve = times;
            f.hw_curve_cpu = times;
          } catch (e2) {}
          // H08 lite: clock / time origin surface
          try {
            f.perf_now = performance.now();
            f.time_origin = performance.timeOrigin || null;
            f.date_now_skew_ms =
              Date.now() - (performance.timeOrigin || Date.now()) - performance.now();
          } catch (e3) {}
          enqueue(ctx, "B17_hw_physical", f, 58, "main");
        });
    },
  });
  register("B18_webgpu", {
    priority: 56,
    schedule: "dynamic",
    batch_id: "B18_webgpu",
    layer: "hard",
    run: function (ctx) {
      var f = {
        webgpu_algo: "gr_webgpu_v3_compute_residual",
        webgpu_compute_algo: "gr_webgpu_compute_residual_v1",
        pohw_direction: "H06",
        collected_at: Date.now(),
        webgpu_available: !!(navigator.gpu),
        is_secure_context: typeof isSecureContext !== "undefined" ? !!isSecureContext : null,
      };
      // iss/67 B1 scaffold: derive seed_k for injection into compute residual.
      // Only mark challenge_used after seed is actually mixed into the WGSL path
      // (session-id alone must not promote fixed residual to silicon V).
      try {
        // iss/69 U6: true challenge = server/boot challenge seed only. ctx.seed /
        // session id remain seed *material* (fallback) but must NOT promote ar to V
        // (server truthy_challenge gate keys off webgpu_challenge_seed_used).
        var b18Seed =
          (ctx && ctx.challenge_seed) ||
          (global.__GR_CHALLENGE_SEED__ != null ? global.__GR_CHALLENGE_SEED__ : null) ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.challenge_seed) ||
          null;
        var b18SeedFallback =
          (ctx && (ctx.seed || ctx.session_id)) || global.__GR_SESSION_ID__ || null;
        var b18SeedSrc = b18Seed != null && b18Seed !== "" ? b18Seed : b18SeedFallback;
        if (b18SeedSrc != null && b18SeedSrc !== "") {
          f.webgpu_challenge_seed_src = String(b18SeedSrc).slice(0, 48);
          var seedNumB18 =
            typeof b18SeedSrc === "number"
              ? b18SeedSrc
              : parseInt(String(b18SeedSrc).replace(/\D/g, "").slice(0, 8) || "0", 10);
          f.webgpu_seed_k =
            Math.round((((seedNumB18 % 9973) / 9973) * 3.1 + 0.17) * 1e6) / 1e6;
          // true challenge only when server/boot challenge_seed present (not bare session)
          f.webgpu_challenge_seed_used = !!(b18Seed != null && b18Seed !== "");
          f.challenge_seed_used = f.webgpu_challenge_seed_used;
        } else {
          f.webgpu_challenge_seed_used = false;
          f.webgpu_seed_k = 0.17;
          f.webgpu_challenge_seed_skip = "no_challenge_or_session_seed";
        }
      } catch (eB18Seed) {
        f.webgpu_challenge_seed_used = false;
        f.webgpu_seed_k = 0.17;
      }
      function packLimits(lim) {
        if (!lim) return {};
        var keys = [
          "maxTextureDimension1D",
          "maxTextureDimension2D",
          "maxTextureDimension3D",
          "maxTextureArrayLayers",
          "maxBindGroups",
          "maxBindingsPerBindGroup",
          "maxDynamicUniformBuffersPerPipelineLayout",
          "maxDynamicStorageBuffersPerPipelineLayout",
          "maxSampledTexturesPerShaderStage",
          "maxSamplersPerShaderStage",
          "maxStorageBuffersPerShaderStage",
          "maxStorageTexturesPerShaderStage",
          "maxUniformBuffersPerShaderStage",
          "maxUniformBufferBindingSize",
          "maxStorageBufferBindingSize",
          "minUniformBufferOffsetAlignment",
          "minStorageBufferOffsetAlignment",
          "maxVertexBuffers",
          "maxBufferSize",
          "maxVertexAttributes",
          "maxVertexBufferArrayStride",
          "maxInterStageShaderComponents",
          "maxComputeWorkgroupStorageSize",
          "maxComputeInvocationsPerWorkgroup",
          "maxComputeWorkgroupSizeX",
          "maxComputeWorkgroupSizeY",
          "maxComputeWorkgroupSizeZ",
          "maxComputeWorkgroupsPerDimension",
        ];
        var out = {};
        keys.forEach(function (k) {
          try {
            if (lim[k] != null) out[k] = lim[k];
          } catch (e) {}
        });
        return out;
      }
      function finish(adapter, tag) {
        if (!adapter) {
          f["webgpu_adapter_" + tag] = null;
          return adapter;
        }
        f["webgpu_adapter_" + tag] = true;
        try {
          f["webgpu_features_" + tag] = adapter.features
            ? Array.from(adapter.features).slice(0, 48)
            : [];
        } catch (eF) {
          f["webgpu_features_" + tag] = [];
        }
        try {
          f["webgpu_limits_" + tag] = packLimits(adapter.limits || {});
        } catch (eL) {}
        try {
          if (adapter.info) {
            f["webgpu_info_" + tag] = {
              vendor: adapter.info.vendor || null,
              architecture: adapter.info.architecture || null,
              device: adapter.info.device || null,
              description: adapter.info.description || null,
            };
          }
        } catch (eI) {}
        try {
          f["webgpu_is_fallback_" + tag] =
            adapter.isFallbackAdapter != null ? !!adapter.isFallbackAdapter : null;
        } catch (eFb) {}
        return adapter;
      }
      /** Silicon-level compute residual: dependent float chain + true buffer readback. */
      function runComputeResidual(adapter) {
        if (!adapter) {
          f.webgpu_compute_skip = "no_adapter";
          return Promise.resolve(null);
        }
        var t0 =
          typeof performance !== "undefined" && performance.now
            ? performance.now()
            : Date.now();
        return adapter
          .requestDevice()
          .then(function (device) {
            var n = 256;
            var curveBins = 32;
            var bufSize = n * 4;
            var storage = device.createBuffer({
              size: bufSize,
              usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
            });
            var staging = device.createBuffer({
              size: bufSize,
              usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
            });
            // ULP-amplifying dependent chain — true silicon numeric response surface.
            // iss/67 B18v2: mix webgpu_seed_k into init so challenge/session seed changes
            // the residual surface (not a fixed class template).
            var sk = typeof f.webgpu_seed_k === "number" && isFinite(f.webgpu_seed_k) ? f.webgpu_seed_k : 0.17;
            var code =
              "@group(0) @binding(0) var<storage, read_write> out_buf: array<u32>;\n" +
              "@compute @workgroup_size(64)\n" +
              "fn main(@builtin(global_invocation_id) gid: vec3<u32>) {\n" +
              "  let i = gid.x;\n" +
              "  if (i >= " +
              n +
              "u) { return; }\n" +
              "  let sk = " +
              sk.toFixed(8) +
              "f;\n" +
              "  var x = f32(i) * 0.001000119 + 1.0000001 + sk * 1e-5;\n" +
              "  var y = f32(i) * 0.000700053 + 1.0000003 + sk * 7e-6;\n" +
              "  var a = x;\n" +
              "  var k = 0u;\n" +
              "  for (k = 0u; k < 48u; k = k + 1u) {\n" +
              "    a = a * (1.000000119 + y * 1e-9) + y * 1e-8 + sk * 1e-10;\n" +
              "    a = a - floor(a * 0.999999881) * 1.000000059;\n" +
              "    a = fract(a * 1.000000013 + sin(y * 0.01 + f32(k) * 0.017 + sk) * 1e-7);\n" +
              "    y = fract(y * 1.0000007 + a * 0.000013);\n" +
              "  }\n" +
              "  let b = fract(a * 97.0 + y * 0.13 + sk * 0.001);\n" +
              "  out_buf[i] = u32(b * 4294967295.0);\n" +
              "}\n";
            f.webgpu_seed_injected = true;
            var shader = device.createShaderModule({ code: code });
            var pipeline = device.createComputePipeline({
              layout: "auto",
              compute: { module: shader, entryPoint: "main" },
            });
            var bind = device.createBindGroup({
              layout: pipeline.getBindGroupLayout(0),
              entries: [{ binding: 0, resource: { buffer: storage } }],
            });
            var enc = device.createCommandEncoder();
            var pass = enc.beginComputePass();
            pass.setPipeline(pipeline);
            pass.setBindGroup(0, bind);
            pass.dispatchWorkgroups(Math.ceil(n / 64));
            pass.end();
            enc.copyBufferToBuffer(storage, 0, staging, 0, bufSize);
            device.queue.submit([enc.finish()]);
            return device.queue.onSubmittedWorkDone
              ? device.queue.onSubmittedWorkDone().then(function () {
                  return {
                    device: device,
                    staging: staging,
                    storage: storage,
                    pipeline: pipeline,
                    bind: bind,
                    n: n,
                    curveBins: curveBins,
                    t0: t0,
                  };
                })
              : Promise.resolve({
                  device: device,
                  staging: staging,
                  storage: storage,
                  pipeline: pipeline,
                  bind: bind,
                  n: n,
                  curveBins: curveBins,
                  t0: t0,
                });
          })
          .then(function (ctx2) {
            if (!ctx2) return null;
            return ctx2.staging.mapAsync(GPUMapMode.READ).then(function () {
              var data = new Uint32Array(ctx2.staging.getMappedRange().slice(0));
              ctx2.staging.unmap();
              // iss/69 U7: device teardown moved after multi-sample timing (see tail).
              var hist16 = new Array(16);
              var i;
              for (i = 0; i < 16; i++) hist16[i] = 0;
              var curve = new Array(ctx2.curveBins);
              for (i = 0; i < ctx2.curveBins; i++) curve[i] = 0;
              var sum = 0;
              var sum2 = 0;
              for (i = 0; i < data.length; i++) {
                var v = data[i] >>> 0;
                hist16[v % 16]++;
                var norm = v / 4294967295.0;
                curve[i % ctx2.curveBins] += norm;
                sum += norm;
                sum2 += norm * norm;
              }
              var per = data.length / ctx2.curveBins;
              for (i = 0; i < ctx2.curveBins; i++) {
                curve[i] = Math.round((curve[i] / per) * 1e6) / 1e6;
              }
              var nn = data.length;
              var mean = sum / nn;
              var std = Math.sqrt(Math.max(0, sum2 / nn - mean * mean));
              var t1 =
                typeof performance !== "undefined" && performance.now
                  ? performance.now()
                  : Date.now();
              f.hw_curve_webgpu = curve;
              f.webgpu_compute_hist16 = hist16;
              f.webgpu_compute_mean = Math.round(mean * 1e6) / 1e6;
              f.webgpu_compute_std = Math.round(std * 1e6) / 1e6;
              f.webgpu_compute_n = nn;
              f.webgpu_compute_ms = Math.round((t1 - ctx2.t0) * 1000) / 1000;
              // Diagnostic only: single wall-clock sample is NOT silicon EU timing V.
              // Server timing_v requires multi-sample curve / dispatch_timings_ms≥2.
              f.webgpu_compute_wall_ms = f.webgpu_compute_ms;
              f.webgpu_compute_ok = true;
              f.webgpu_compute_entropy_hint = std > 0.01;
              f.webgpu_compute_head8 = Array.prototype.slice.call(data, 0, 8);
              // iss/69 U7 (B18v2 timing-V): 3 warm dispatch samples before teardown.
              // Server `ar_has_timing_v` promotes only on multi-sample + low CV.
              return sampleWebgpuTimings(ctx2).then(function (timing) {
                if (timing) {
                  f.webgpu_dispatch_timings_ms = timing.samples;
                  f.webgpu_compute_timing_curve = timing.samples.slice();
                  f.webgpu_timing_median_ms = timing.median;
                  f.webgpu_timing_iqr_ms = timing.iqr;
                  f.webgpu_timing_cv = timing.cv;
                  f.webgpu_timing_quality = timing.quality;
                  f.webgpu_timing_n = timing.samples.length;
                } else {
                  f.webgpu_timing_skip = "no_onSubmittedWorkDone_or_error";
                }
                // iss/73–74 B18v2: EU atomic contention (LockedApart/vektort13) before teardown.
                return sampleWebgpuEuAtomic(ctx2).then(function (eu) {
                  if (eu && eu.ok) {
                    f.webgpu_eu_timing_curve = eu.curve;
                    f.webgpu_eu_workgroup_incs = eu.incs;
                    f.webgpu_eu_median = eu.median;
                    f.webgpu_eu_iqr = eu.iqr;
                    f.webgpu_eu_cv = eu.cv;
                    f.webgpu_eu_ok = true;
                    f.webgpu_eu_seed_k = eu.seed_k;
                    f.webgpu_eu_n_workgroups = eu.n_wg;
                  } else if (eu) {
                    f.webgpu_eu_ok = false;
                    f.webgpu_eu_skip = eu.skip || "eu_failed";
                  } else {
                    f.webgpu_eu_skip = "eu_unavailable";
                  }
                  try {
                    ctx2.device.destroy();
                  } catch (eD) {}
                  return curve;
                });
              });
            });
          })
          .catch(function (e) {
            f.webgpu_compute_ok = false;
            f.webgpu_compute_error = String((e && e.message) || e);
            f.webgpu_compute_skip = "compute_failed";
            return null;
          });
      }
      /**
       * iss/69 U7 (B18v2 timing-V): warm dispatch timing samples.
       * Reuses the f32 device/pipeline (no extra requestAdapter — product rule).
       * No timestamp-query required: onSubmittedWorkDone wall samples, median/IQR/CV.
       */
      function sampleWebgpuTimings(ctx2) {
        if (!ctx2 || !ctx2.device || !ctx2.pipeline || !ctx2.bind || !ctx2.storage || !ctx2.staging) {
          return Promise.resolve(null);
        }
        if (!ctx2.device.queue || !ctx2.device.queue.onSubmittedWorkDone) {
          return Promise.resolve(null);
        }
        var samples = [];
        var rounds = 3;
        function oneRound() {
          var tS =
            typeof performance !== "undefined" && performance.now
              ? performance.now()
              : Date.now();
          var enc2 = ctx2.device.createCommandEncoder();
          var pass2 = enc2.beginComputePass();
          pass2.setPipeline(ctx2.pipeline);
          pass2.setBindGroup(0, ctx2.bind);
          pass2.dispatchWorkgroups(Math.ceil(ctx2.n / 64));
          pass2.end();
          enc2.copyBufferToBuffer(ctx2.storage, 0, ctx2.staging, 0, ctx2.n * 4);
          ctx2.device.queue.submit([enc2.finish()]);
          return ctx2.device.queue
            .onSubmittedWorkDone()
            .then(function () {
              samples.push(
                Math.round(
                  ((typeof performance !== "undefined" && performance.now
                    ? performance.now()
                    : Date.now()) -
                    tS) *
                    1000
                ) / 1000
              );
              if (samples.length < rounds) {
                return oneRound();
              }
              var sorted = samples.slice().sort(function (a, b) {
                return a - b;
              });
              var med = sorted[Math.floor(sorted.length / 2)];
              var q1 = sorted[Math.floor((sorted.length - 1) / 4)];
              var q3 = sorted[Math.floor((3 * sorted.length - 1) / 4)];
              var mean = 0;
              var si;
              for (si = 0; si < sorted.length; si++) mean += sorted[si];
              mean = mean / sorted.length;
              var vr = 0;
              for (si = 0; si < sorted.length; si++) {
                var dd = sorted[si] - mean;
                vr += dd * dd;
              }
              vr = vr / sorted.length;
              var cv = mean > 0 ? Math.sqrt(vr) / mean : 0;
              return {
                samples: sorted,
                median: Math.round(med * 1000) / 1000,
                iqr: Math.round((q3 - q1) * 1000) / 1000,
                cv: Math.round(cv * 10000) / 10000,
                quality:
                  typeof document !== "undefined" && document.hidden ? "throttled" : "ok",
              };
            })
            .catch(function () {
              return null;
            });
        }
        return oneRound();
      }
      /**
       * iss/73–74 B18v2 EU atomic contention (LockedApart / vektort13 style).
       * 32 workgroups contend on one atomic counter; per-workgroup increments
       * form a curve. Challenge seed skews loop count for replay immunity.
       * Emits webgpu_eu_timing_curve (+ diagnostics). Soft-fail on missing GPU.
       */
      function sampleWebgpuEuAtomic(ctx2) {
        if (!ctx2 || !ctx2.device) {
          return Promise.resolve({ ok: false, skip: "no_device" });
        }
        try {
          var sk =
            typeof f.webgpu_seed_k === "number" && isFinite(f.webgpu_seed_k)
              ? f.webgpu_seed_k
              : 0;
          var nWg = 32;
          var loops = 256 + Math.floor((Math.abs(sk) % 1) * 64);
          var seedU = Math.floor((Math.abs(sk) * 1e6) % 65536);
          var code =
            "struct Ctr { v: atomic<u32> }\n" +
            "@group(0) @binding(0) var<storage, read_write> ctr: Ctr;\n" +
            "@group(0) @binding(1) var<storage, read_write> outs: array<u32>;\n" +
            "@compute @workgroup_size(64)\n" +
            "fn main(@builtin(workgroup_id) wid: vec3u, @builtin(local_invocation_id) lid: vec3u) {\n" +
            "  if (lid.x != 0u) { return; }\n" +
            "  var i: u32 = 0u;\n" +
            "  var local: u32 = 0u;\n" +
            "  var junk: u32 = 0u;\n" +
            "  let lim: u32 = " +
            loops +
            "u;\n" +
            "  loop {\n" +
            "    if (i >= lim) { break; }\n" +
            "    local = local + 1u;\n" +
            "    atomicAdd(&ctr.v, 1u);\n" +
            "    junk = i * 1664525u + " +
            seedU +
            "u;\n" +
            "    junk = junk ^ (junk >> 13u);\n" +
            "    i = i + 1u;\n" +
            "  }\n" +
            "  outs[wid.x] = local + (junk & 3u);\n" +
            "}\n";
          var module = ctx2.device.createShaderModule({ code: code });
          var pipeline = ctx2.device.createComputePipeline({
            layout: "auto",
            compute: { module: module, entryPoint: "main" },
          });
          var ctrBuf = ctx2.device.createBuffer({
            size: 4,
            usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST,
          });
          var outBuf = ctx2.device.createBuffer({
            size: nWg * 4,
            usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
          });
          var stage = ctx2.device.createBuffer({
            size: nWg * 4,
            usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
          });
          var bind = ctx2.device.createBindGroup({
            layout: pipeline.getBindGroupLayout(0),
            entries: [
              { binding: 0, resource: { buffer: ctrBuf } },
              { binding: 1, resource: { buffer: outBuf } },
            ],
          });
          ctx2.device.queue.writeBuffer(ctrBuf, 0, new Uint32Array([0]));
          var enc = ctx2.device.createCommandEncoder();
          var pass = enc.beginComputePass();
          pass.setPipeline(pipeline);
          pass.setBindGroup(0, bind);
          pass.dispatchWorkgroups(nWg);
          pass.end();
          enc.copyBufferToBuffer(outBuf, 0, stage, 0, nWg * 4);
          ctx2.device.queue.submit([enc.finish()]);
          return stage
            .mapAsync(GPUMapMode.READ)
            .then(function () {
              var data = new Uint32Array(stage.getMappedRange().slice(0));
              stage.unmap();
              var incs = Array.prototype.slice.call(data);
              var curve = incs.map(function (x) {
                return x / loops;
              });
              var sorted = curve.slice().sort(function (a, b) {
                return a - b;
              });
              var med = sorted[Math.floor(sorted.length / 2)];
              var q1 = sorted[Math.floor((sorted.length - 1) / 4)];
              var q3 = sorted[Math.floor((3 * sorted.length - 1) / 4)];
              var mean = 0;
              var i;
              for (i = 0; i < sorted.length; i++) mean += sorted[i];
              mean /= sorted.length;
              var vr = 0;
              for (i = 0; i < sorted.length; i++) {
                var d = sorted[i] - mean;
                vr += d * d;
              }
              vr /= sorted.length;
              var cv = mean > 0 ? Math.sqrt(vr) / mean : 0;
              try {
                ctrBuf.destroy();
                outBuf.destroy();
                stage.destroy();
              } catch (eZ) {}
              return {
                ok: true,
                curve: curve,
                incs: incs,
                median: Math.round(med * 1e6) / 1e6,
                iqr: Math.round((q3 - q1) * 1e6) / 1e6,
                cv: Math.round(cv * 1e4) / 1e4,
                seed_k: sk,
                n_wg: nWg,
              };
            })
            .catch(function (e) {
              return { ok: false, skip: String((e && e.message) || e || "map_failed") };
            });
        } catch (e) {
          return Promise.resolve({
            ok: false,
            skip: String((e && e.message) || e || "eu_exception"),
          });
        }
      }

      /**
       * iss/54 P5 — WebGPU f16 residual (shader-f16). Stronger silicon channel than f32
       * (vendor subnormal / transcendental tables less normalized). Honest skip when
       * feature absent. Uses same adapter, serial after f32 compute.
       */
      function runComputeResidualF16(adapter) {
        if (!adapter) {
          f.webgpu_f16_skip = "no_adapter";
          return Promise.resolve(null);
        }
        var hasF16 = false;
        try {
          hasF16 = !!(adapter.features && adapter.features.has && adapter.features.has("shader-f16"));
        } catch (eF) {
          hasF16 = false;
        }
        f.webgpu_shader_f16_feature = hasF16;
        if (!hasF16) {
          f.webgpu_f16_skip = "no_shader_f16_feature";
          return Promise.resolve(null);
        }
        var t0 =
          typeof performance !== "undefined" && performance.now
            ? performance.now()
            : Date.now();
        return adapter
          .requestDevice({ requiredFeatures: ["shader-f16"] })
          .then(function (device) {
            var n = 256;
            var curveBins = 32;
            var bufSize = n * 4;
            var storage = device.createBuffer({
              size: bufSize,
              usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
            });
            var staging = device.createBuffer({
              size: bufSize,
              usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
            });
            // iss/69 U6: f16 path gets the same challenge/session seed so the primary
            // Lane-S material is no longer a fixed class template. Magnitudes chosen
            // above f16 epsilon (~1e-3 at 1.0) so every seed changes the rounding trail.
            var sk =
              typeof f.webgpu_seed_k === "number" && isFinite(f.webgpu_seed_k)
                ? f.webgpu_seed_k
                : 0.17;
            // f16 dependent chain — half-precision rounding / FTZ differences
            var code =
              "enable f16;\n" +
              "@group(0) @binding(0) var<storage, read_write> out_buf: array<u32>;\n" +
              "@compute @workgroup_size(64)\n" +
              "fn main(@builtin(global_invocation_id) gid: vec3<u32>) {\n" +
              "  let i = gid.x;\n" +
              "  if (i >= " +
              n +
              "u) { return; }\n" +
              "  let sk = " +
              sk.toFixed(8) +
              "f;\n" +
              "  var x = f16(f32(i) * 0.001000119 + 1.0000001 + sk * 1e-2);\n" +
              "  var y = f16(f32(i) * 0.000700053 + 1.0000003 + sk * 7e-3);\n" +
              "  var a = x;\n" +
              "  var k = 0u;\n" +
              "  for (k = 0u; k < 48u; k = k + 1u) {\n" +
              "    a = a * (f16(1.000000119) + y * f16(1e-5)) + y * f16(1e-4) + f16(sk) * f16(1e-3);\n" +
              "    a = a - floor(a * f16(0.9999)) * f16(1.0001);\n" +
              "    a = fract(a * f16(1.0001) + sin(f32(y) * 0.01 + f32(k) * 0.017 + f32(sk) * 0.5) * f16(1e-3));\n" +
              "    y = fract(y * f16(1.0007) + a * f16(0.0013));\n" +
              "  }\n" +
              "  let b = fract(f32(a) * 97.0 + f32(y) * 0.13);\n" +
              "  out_buf[i] = u32(b * 4294967295.0);\n" +
              "}\n";
            f.webgpu_f16_seed_injected = true;
            f.webgpu_f16_seed_k = Math.round(sk * 1e6) / 1e6;
            var shader = device.createShaderModule({ code: code });
            var pipeline = device.createComputePipeline({
              layout: "auto",
              compute: { module: shader, entryPoint: "main" },
            });
            var bind = device.createBindGroup({
              layout: pipeline.getBindGroupLayout(0),
              entries: [{ binding: 0, resource: { buffer: storage } }],
            });
            var enc = device.createCommandEncoder();
            var pass = enc.beginComputePass();
            pass.setPipeline(pipeline);
            pass.setBindGroup(0, bind);
            pass.dispatchWorkgroups(Math.ceil(n / 64));
            pass.end();
            enc.copyBufferToBuffer(storage, 0, staging, 0, bufSize);
            device.queue.submit([enc.finish()]);
            return device.queue.onSubmittedWorkDone
              ? device.queue.onSubmittedWorkDone().then(function () {
                  return {
                    device: device,
                    staging: staging,
                    storage: storage,
                    pipeline: pipeline,
                    bind: bind,
                    n: n,
                    curveBins: curveBins,
                    t0: t0,
                  };
                })
              : Promise.resolve({
                  device: device,
                  staging: staging,
                  storage: storage,
                  pipeline: pipeline,
                  bind: bind,
                  n: n,
                  curveBins: curveBins,
                  t0: t0,
                });
          })
          .then(function (ctx2) {
            if (!ctx2) return null;
            return ctx2.staging.mapAsync(GPUMapMode.READ).then(function () {
              var data = new Uint32Array(ctx2.staging.getMappedRange().slice(0));
              ctx2.staging.unmap();
              try {
                ctx2.device.destroy();
              } catch (eD) {}
              var hist16 = new Array(16);
              var i;
              for (i = 0; i < 16; i++) hist16[i] = 0;
              var curve = new Array(ctx2.curveBins);
              for (i = 0; i < ctx2.curveBins; i++) curve[i] = 0;
              var sum = 0;
              var sum2 = 0;
              for (i = 0; i < data.length; i++) {
                var v = data[i] >>> 0;
                hist16[v % 16]++;
                var norm = v / 4294967295.0;
                curve[i % ctx2.curveBins] += norm;
                sum += norm;
                sum2 += norm * norm;
              }
              var per = data.length / ctx2.curveBins;
              for (i = 0; i < ctx2.curveBins; i++) {
                curve[i] = Math.round((curve[i] / per) * 1e6) / 1e6;
              }
              var nn = data.length;
              var mean = sum / nn;
              var std = Math.sqrt(Math.max(0, sum2 / nn - mean * mean));
              var t1 =
                typeof performance !== "undefined" && performance.now
                  ? performance.now()
                  : Date.now();
              f.hw_curve_webgpu_f16 = curve;
              f.webgpu_f16_hist16 = hist16;
              f.webgpu_f16_mean = Math.round(mean * 1e6) / 1e6;
              f.webgpu_f16_std = Math.round(std * 1e6) / 1e6;
              f.webgpu_f16_ms = Math.round((t1 - ctx2.t0) * 1000) / 1000;
              f.webgpu_f16_ok = true;
              f.webgpu_f16_head8 = Array.prototype.slice.call(data, 0, 8);
              f.webgpu_f16_algo = "gr_webgpu_f16_residual_v1";
              // Prefer f16 as primary Lane-S when healthier entropy
              if (
                !f.hw_curve_webgpu ||
                (f.webgpu_compute_std != null && std > Number(f.webgpu_compute_std) * 1.05)
              ) {
                // keep f32 primary; f16 is secondary material — never overwrite without note
              }
              return curve;
            });
          })
          .catch(function (e) {
            f.webgpu_f16_ok = false;
            f.webgpu_f16_error = String((e && e.message) || e);
            f.webgpu_f16_skip = "f16_compute_failed";
            return null;
          });
      }
      if (!navigator.gpu) {
        f.webgpu_skip = "need_webgpu_or_secure_context";
        f.webgpu_compute_skip = "no_gpu_api";
        f.data_ok = true;
        f.webgpu_adapter_surface = "none";
        f.webgpu_limits_hash = f.webgpu_limits_hash || "none";
        f.webgpu_features_hash = f.webgpu_features_hash || "none";
        enqueue(ctx, "B18_webgpu", f, 56, "main");
        return;
      }
      // Product: at most ONE requestAdapter per session (Chrome logs "No available adapters" each call).
      // Dual high/low preference only when first adapter succeeds.
      var bestAdapter = null;
      function onceAdapter(opt) {
        if (global.__GR_WEBGPU_NO_ADAPTER__) return Promise.resolve(null);
        if (global.__GR_WEBGPU_ADAPTER__ && !opt) return Promise.resolve(global.__GR_WEBGPU_ADAPTER__);
        return navigator.gpu.requestAdapter(opt || {}).then(
          function (ad) {
            if (!ad) {
              global.__GR_WEBGPU_NO_ADAPTER__ = 1;
              return null;
            }
            if (!opt || !opt.powerPreference) global.__GR_WEBGPU_ADAPTER__ = ad;
            return ad;
          },
          function () {
            global.__GR_WEBGPU_NO_ADAPTER__ = 1;
            return null;
          }
        );
      }
      return onceAdapter({})
        .then(function (ad0) {
          finish(ad0, "default");
          if (ad0) bestAdapter = ad0;
          f.webgpu_adapter_high = null;
          f.webgpu_adapter_low = null;
          // Skip high/low re-request when default is null (avoids 2 more console lines).
          if (!ad0) return null;
          // Optional dual-pref only when default worked — still one extra max under product pressure.
          return onceAdapter({ powerPreference: "high-performance" }).then(function (adH) {
            finish(adH, "high");
            if (adH) bestAdapter = adH;
          });
        })
        .then(function () {
          f.webgpu_adapter = !!(f.webgpu_adapter_default || f.webgpu_adapter_high || f.webgpu_adapter_low);
          f.webgpu_limits = f.webgpu_limits_default || f.webgpu_limits_high || f.webgpu_limits_low || {};
          f.webgpu_features = f.webgpu_features_default || f.webgpu_features_high || [];
          f.webgpu_dual_adapter_diff =
            !!(f.webgpu_adapter_high && f.webgpu_adapter_default) &&
            JSON.stringify(f.webgpu_limits_high || {}) !== JSON.stringify(f.webgpu_limits_default || {});
          f.webgpu_limits_n = Object.keys(f.webgpu_limits || {}).length;
          f.webgpu_limits_hash = simpleHash(JSON.stringify(f.webgpu_limits || {})).slice(0, 12);
          if (!f.webgpu_adapter) {
            f.webgpu_skip = "no_adapter";
            f.webgpu_compute_skip = "no_adapter";
            f.data_ok = true;
            f.webgpu_adapter_surface = f.webgpu_adapter_surface || "no_adapter";
            enqueue(ctx, "B18_webgpu", f, 56, "main");
            return null;
          }
          // Serial: f32 compute then f16 (same GPU resource class — no parallel adapters)
          return runComputeResidual(bestAdapter).then(function () {
            return runComputeResidualF16(bestAdapter);
          });
        })
        .then(function () {
          f.data_ok = f.webgpu_adapter === true;
          // Prefer compute residual when present; adapter inventory alone still ok
          if (f.webgpu_compute_ok || f.webgpu_f16_ok) f.data_ok = true;
          f.webgpu_algo = "gr_webgpu_v4_f32_f16";
          enqueue(ctx, "B18_webgpu", f, 56, "main");
        })
        .catch(function (e) {
          f.webgpu_error = String((e && e.message) || e);
          f.data_ok = f.webgpu_adapter === true;
          enqueue(ctx, "B18_webgpu", f, 56, "main");
        });
    },
  });
  register("B19_eme_media", {
    priority: 48,
    schedule: "dynamic",
    batch_id: "B19_eme_media",
    layer: "deep",
    run: function (ctx) {
      var f = {
        eme_algo: "gr_eme_h10_v2",
        pohw_direction: "H10",
        collected_at: Date.now(),
        media_capabilities: !!(navigator.mediaCapabilities && navigator.mediaCapabilities.decodingInfo),
        request_media_key_system: !!(navigator.requestMediaKeySystemAccess),
      };
      var v = document.createElement("video");
      var a = document.createElement("audio");
      // H10 object-depth matrix (VM almost always loses rare codecs)
      var codecs = [
        ["video/mp4", 'video/mp4; codecs="avc1.42E01E"'],
        ["video/mp4_high", 'video/mp4; codecs="avc1.640028"'],
        ["video/mp4_hevc", 'video/mp4; codecs="hev1.1.6.L93.B0"'],
        ["video/mp4_hevc_main10", 'video/mp4; codecs="hvc1.1.6.L120.90"'],
        ["video/mp4_av1", 'video/mp4; codecs="av01.0.05M.08"'],
        ["video/mp4_av1_10", 'video/mp4; codecs="av01.0.12M.10"'],
        ["video/webm_vp8", 'video/webm; codecs="vp8, vorbis"'],
        // Use RFC6381 vp09 — bare "vp9" is ambiguous and floods Chrome console.
        ["video/webm_vp9", 'video/webm; codecs="vp09.00.10.08"'],
        ["video/webm_vp9_profile2", 'video/webm; codecs="vp09.02.10.10"'],
        ["video/ogg_theora", 'video/ogg; codecs="theora"'],
        ["audio/mp4_aac", 'audio/mp4; codecs="mp4a.40.2"'],
        ["audio/mp4_mp3", 'audio/mp4; codecs="mp3"'],
        ["audio/webm_opus", 'audio/webm; codecs="opus"'],
        ["audio/ogg", 'audio/ogg; codecs="vorbis"'],
        ["audio/wav", "audio/wav"],
        ["audio/flac", "audio/flac"],
      ];
      f.canplay_matrix = {};
      f.codec_matrix = {};
      f.codec_probably_n = 0;
      f.codec_maybe_n = 0;
      f.codec_empty_n = 0;
      codecs.forEach(function (pair) {
        var key = pair[0];
        var type = pair[1];
        var r = "";
        try {
          r = (key.indexOf("audio") === 0 ? a : v).canPlayType(type) || "";
        } catch (e) {
          r = "error";
        }
        f.canplay_matrix[key] = r;
        f.codec_matrix[key] = r === "probably" ? 2 : r === "maybe" ? 1 : 0;
        if (r === "probably") f.codec_probably_n++;
        else if (r === "maybe") f.codec_maybe_n++;
        else f.codec_empty_n++;
      });
      f.canplay_mp4 = f.canplay_matrix["video/mp4"] || "";
      f.canplay_webm = f.canplay_matrix["video/webm_vp8"] || "";
      f.canplay_hevc = f.canplay_matrix["video/mp4_hevc"] || "";
      f.canplay_av1 = f.canplay_matrix["video/mp4_av1"] || "";
      f.codec_support_score = Object.keys(f.codec_matrix).reduce(function (s, k) {
        return s + f.codec_matrix[k];
      }, 0);
      f.codec_matrix_n = Object.keys(f.codec_matrix).length;
      f.codec_matrix_hash = simpleHash(
        Object.keys(f.codec_matrix)
          .sort()
          .map(function (k) {
            return k + "=" + f.codec_matrix[k];
          })
          .join("|")
      ).slice(0, 12);
      // Structured RTCRtpReceiver.getCapabilities (hardware codec table; not canPlayType string).
      try {
        f.rtc_receiver_caps = { audio: [], video: [] };
        if (typeof RTCRtpReceiver !== "undefined" && RTCRtpReceiver.getCapabilities) {
          ["audio", "video"].forEach(function (kind) {
            try {
              var caps = RTCRtpReceiver.getCapabilities(kind);
              var codecs = (caps && caps.codecs) || [];
              f.rtc_receiver_caps[kind] = codecs.slice(0, 48).map(function (c) {
                return {
                  mimeType: c.mimeType || "",
                  clockRate: c.clockRate != null ? c.clockRate : null,
                  channels: c.channels != null ? c.channels : null,
                  sdpFmtpLine: c.sdpFmtpLine ? String(c.sdpFmtpLine).slice(0, 120) : "",
                };
              });
            } catch (eC) {
              f.rtc_receiver_caps[kind + "_err"] = String((eC && eC.message) || eC);
            }
          });
          f.rtc_receiver_caps_n =
            (f.rtc_receiver_caps.audio || []).length + (f.rtc_receiver_caps.video || []).length;
          f.rtc_receiver_caps_hash = simpleHash(
            JSON.stringify(f.rtc_receiver_caps.audio || []) +
              "|" +
              JSON.stringify(f.rtc_receiver_caps.video || [])
          ).slice(0, 12);
        } else {
          f.rtc_receiver_caps_skip = "no_getCapabilities";
        }
      } catch (eRtc) {
        f.rtc_receiver_caps_err = String((eRtc && eRtc.message) || eRtc);
      }
      function finish() {
        f.eme_supported_n = 0;
        f.eme_unsupported_n = 0;
        if (f.eme_systems) {
          Object.keys(f.eme_systems).forEach(function (k) {
            if (f.eme_systems[k] === "supported") f.eme_supported_n++;
            else f.eme_unsupported_n++;
          });
        }
        f.eme_widevine = f.eme_systems && f.eme_systems["com.widevine.alpha"] === "supported";
        f.eme_clearkey = f.eme_systems && f.eme_systems["org.w3.clearkey"] === "supported";
        // virt_hint: desktop Chrome-class usually has some probably codecs; total zero is suspicious
        f.codec_virt_hint =
          f.codec_support_score === 0 && f.codec_matrix_n >= 8 ? true : false;
        f.data_ok =
          f.codec_support_score > 0 ||
          !!(f.eme_systems && Object.keys(f.eme_systems).length) ||
          f.codec_matrix_n > 0;
        enqueue(ctx, "B19_eme_media", f, 48, "main");
      }
      if (!navigator.requestMediaKeySystemAccess) {
        f.eme_skip = "need_media_eme";
        f.eme_systems = {};
        finish();
        return;
      }
      var configs = [
        {
          initDataTypes: ["cenc"],
          videoCapabilities: [{ contentType: 'video/mp4; codecs="avc1.42E01E"' }],
        },
      ];
      var systems = [
        "com.widevine.alpha",
        "com.microsoft.playready",
        "com.apple.fps.1_0",
        "org.w3.clearkey",
      ];
      var pending = systems.length;
      f.eme_systems = {};
      systems.forEach(function (sys) {
        navigator
          .requestMediaKeySystemAccess(sys, configs)
          .then(function () {
            f.eme_systems[sys] = "supported";
            pending--;
            if (pending <= 0) finish();
          })
          .catch(function () {
            f.eme_systems[sys] = "unsupported";
            pending--;
            if (pending <= 0) finish();
          });
      });
    },
  });
  register("B20_challenge_seed", {
    priority: 57,
    schedule: "dynamic",
    batch_id: "B20_challenge_seed",
    layer: "hard",
    run: function (ctx) {
      var seed =
        (ctx && (ctx.challenge_seed || ctx.seed)) ||
        (global.__GR_CHALLENGE_SEED__ != null ? global.__GR_CHALLENGE_SEED__ : null) ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.challenge_seed) ||
        null;
      var seedSig =
        (ctx && ctx.challenge_seed_sig) ||
        global.__GR_CHALLENGE_SEED_SIG__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.challenge_seed_sig) ||
        null;
      var seedExp =
        (ctx && ctx.challenge_seed_exp_ms) ||
        global.__GR_CHALLENGE_SEED_EXP__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.challenge_seed_exp_ms) ||
        null;
      var f = {
        challenge_algo: "gr_pohw_v4",
        collected_at: Date.now(),
        has_challenge_seed: seed != null && seed !== "",
        pohw_direction: "H16",
        challenge_seed: seed,
        challenge_seed_sig: seedSig,
        challenge_seed_exp_ms: seedExp,
        session_id: (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || null,
      };
      if (!f.has_challenge_seed) {
        f.challenge_skip = "need_challenge_seed";
        seed = Date.now() % 1000000;
        f.challenge_seed = seed;
      }
      f.challenge_seed_signed = !!(seedSig && seedExp);
      var seedNum =
        typeof seed === "number"
          ? seed
          : parseInt(String(seed).replace(/\D/g, "").slice(0, 8) || "0", 10);
      f.challenge_seed_digest = "cs_" + simpleHash(String(seed)).slice(0, 12);
      function drawResidual(gl, sn, size) {
        function mk(type, src) {
          var sh = gl.createShader(type);
          gl.shaderSource(sh, src);
          gl.compileShader(sh);
          return sh;
        }
        var a = 37.1 + (sn % 97) * 0.01;
        var b = 29.3 + (sn % 53) * 0.01;
        var d = 53.7 + (sn % 31) * 0.01;
        var vs = mk(gl.VERTEX_SHADER, "attribute vec2 p;void main(){gl_Position=vec4(p,0.0,1.0);}");
        var fs = mk(
          gl.FRAGMENT_SHADER,
          "precision mediump float;void main(){vec2 uv=gl_FragCoord.xy*" +
            (1 / size).toFixed(6) +
            ";float n=sin(uv.x*" +
            a.toFixed(4) +
            ")*cos(uv.y*" +
            b.toFixed(4) +
            ")+sin((uv.x+uv.y)*" +
            d.toFixed(4) +
            ");gl_FragColor=vec4(fract(n*0.5+0.5),fract(n*1.7),fract(uv.x*uv.y*9.0),1.0);}"
        );
        var prog = gl.createProgram();
        gl.attachShader(prog, vs);
        gl.attachShader(prog, fs);
        gl.linkProgram(prog);
        gl.useProgram(prog);
        var buf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        gl.bufferData(
          gl.ARRAY_BUFFER,
          new Float32Array([-1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1]),
          gl.STATIC_DRAW
        );
        var loc = gl.getAttribLocation(prog, "p");
        gl.enableVertexAttribArray(loc);
        gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
        gl.viewport(0, 0, size, size);
        var t0 = performance.now();
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        var pix = new Uint8Array(size * size * 4);
        gl.readPixels(0, 0, size, size, gl.RGBA, gl.UNSIGNED_BYTE, pix);
        var wall = performance.now() - t0;
        var sum = 0;
        for (var i = 0; i < pix.length; i += 4) sum += pix[i];
        return {
          mean: Math.round((sum / (pix.length / 4) / 255) * 1e12) / 1e12,
          wall_ms: Math.round(wall * 1000) / 1000,
          sample: [pix[0], pix[4], pix[16], pix[64] || 0],
          size: size,
        };
      }
      try {
        var c = document.createElement("canvas");
        c.width = 128;
        c.height = 128;
        var gl =
          c.getContext("webgl", { preserveDrawingBuffer: true }) ||
          c.getContext("experimental-webgl");
        if (gl) {
          var same = [];
          for (var r = 0; r < 3; r++) same.push(drawResidual(gl, seedNum, 32));
          var alt = drawResidual(gl, seedNum + 7919, 32);
          var sizesLadder = [16, 32, 64];
          var sizeMeans = sizesLadder.map(function (sz) {
            return drawResidual(gl, seedNum, sz);
          });
          f.challenge_residual_mean = same[0].mean;
          f.challenge_repeat_means = same.map(function (x) { return x.mean; });
          f.challenge_wall_ms = same.map(function (x) { return x.wall_ms; });
          f.challenge_alt_mean = alt.mean;
          f.challenge_alt_changed = Math.abs(alt.mean - same[0].mean) > 1e-9;
          f.challenge_size_ladder = sizeMeans.map(function (x) {
            return { size: x.size, mean: x.mean, wall_ms: x.wall_ms };
          });
          var uniq = {};
          same.forEach(function (x) { uniq[String(x.mean)] = 1; });
          f.challenge_unique_digest_n = Object.keys(uniq).length;
          var meanAvg = same.reduce(function (s, x) { return s + x.mean; }, 0) / same.length;
          var varSum = 0;
          same.forEach(function (x) { var d = x.mean - meanAvg; varSum += d * d; });
          f.challenge_avg_cv =
            meanAvg !== 0 ? Math.sqrt(varSum / same.length) / Math.abs(meanAvg) : 0;
          f.challenge_pixel_sample = same[0].sample;
          
          f.noise_suspect =
            f.challenge_avg_cv > 0.08 ||
            (f.challenge_unique_digest_n === 1 && !f.challenge_alt_changed);
          f.seed_digest = f.challenge_seed_digest;
          f.challenge_hist_digest = "ch_" + simpleHash(
            same.map(function (x) { return String(x.mean); }).join("|")
          ).slice(0, 12);
          // Seeded residual/ulp digests for fp_channel replay_hardness
          try {
            var sk = ((seedNum % 9973) / 9973) * 3.1 + 0.17;
            f.seed_k = Math.round(sk * 1e6) / 1e6;
            var sr1 = webglResidualCurveV3fRun({
              size: 64,
              warm_frames: 0,
              force_deriv: false,
              shader_mode: "noderiv",
              seed_k: sk,
              ctx_pref: "webgl",
              release_gl: true,
            });
            var sr2 = webglResidualCurveV3fRun({
              size: 64,
              warm_frames: 0,
              force_deriv: false,
              shader_mode: "noderiv",
              seed_k: sk,
              ctx_pref: "webgl",
              release_gl: true,
            });
            if (sr1 && sr1.mean != null && sr2 && sr2.mean != null) {
              f.seed_residual_means = [sr1.mean, sr2.mean];
              f.seed_replay_agree_0p001 = Math.abs(sr1.mean - sr2.mean) < 0.001;
              f.seed_residual_digest =
                "sr_" +
                simpleHash(
                  String(Math.round(sr1.mean * 1000) / 1000) +
                    "|" +
                    String(Math.round(sr2.mean * 1000) / 1000)
                ).slice(0, 12);
            }
            var su1 = webglResidualCurveV3fRun({
              size: 64,
              warm_frames: 0,
              force_deriv: false,
              shader_mode: "ulp",
              seed_k: sk,
              timing_samples: 4,
              ctx_pref: "webgl",
              release_gl: true,
            });
            var su2 = webglResidualCurveV3fRun({
              size: 64,
              warm_frames: 0,
              force_deriv: false,
              shader_mode: "ulp",
              seed_k: sk + 0.000001,
              timing_samples: 0,
              ctx_pref: "webgl",
              release_gl: true,
            });
            if (su1 && su1.mean != null) {
              f.seed_ulp_means = [su1.mean, su2 && su2.mean];
              f.seed_ulp_agree =
                su2 && su2.mean != null ? Math.abs(su1.mean - su2.mean) < 1e-5 : null;
              f.seed_ulp_digest =
                "su_" +
                simpleHash(String(su1.mean) + "|" + String(su2 && su2.mean)).slice(0, 12);
            }
            try {
              releaseWebglProbeContexts();
            } catch (eRel) {}
          } catch (eSeedB20) {
            f.seed_replay_err = String((eSeedB20 && eSeedB20.message) || eSeedB20);
          }
          f.pohw_triad = {
            seed_digest: f.challenge_seed_digest,
            residual_mean: f.challenge_residual_mean,
            wall_median_ms: same.map(function (x) { return x.wall_ms; }).sort(function (a, b) { return a - b; })[1],
            alt_changed: f.challenge_alt_changed,
            unique_n: f.challenge_unique_digest_n,
            size_means: sizeMeans.map(function (x) { return x.mean; }),
            seed_residual_digest: f.seed_residual_digest || null,
            seed_ulp_digest: f.seed_ulp_digest || null,
          };
        }
      } catch (eC) {
        f.challenge_error = String((eC && eC.message) || eC);
      }
      // Audio challenge surface (seed-bound freq) — D37-style second axis
      try {
        var AC = window.AudioContext || window.webkitAudioContext;
        if (AC) {
          var ac = new AC();
          var sr = ac.sampleRate || 44100;
          var len = 2048;
          var buf = ac.createBuffer(1, len, sr);
          var data = buf.getChannelData(0);
          var freq = 440 + (seedNum % 800);
          for (var i = 0; i < len; i++) {
            data[i] = Math.sin((2 * Math.PI * freq * i) / sr) * 0.5;
          }
          var sumA = 0, sumSq = 0;
          for (var j = 0; j < len; j++) { sumA += data[j]; sumSq += data[j] * data[j]; }
          f.challenge_audio_freq = freq;
          f.challenge_audio_mean = Math.round((sumA / len) * 1e9) / 1e9;
          f.challenge_audio_rms = Math.round(Math.sqrt(sumSq / len) * 1e9) / 1e9;
          if (f.pohw_triad) {
            f.pohw_triad.audio_rms = f.challenge_audio_rms;
            f.pohw_triad.audio_freq = freq;
          }
          try { if (ac.close) { var _cl = ac.close(); if (_cl && _cl.catch) _cl.catch(function(){}); } } catch (_eCl) {}
        }
      } catch (eA) {
        f.challenge_audio_error = String((eA && eA.message) || eA);
      }
      // CPU timing challenge (seed-bound loop)
      try {
        var nOps = 50000 + (seedNum % 10000);
        var t0c = performance.now();
        var acc = seedNum % 97;
        for (var k = 0; k < nOps; k++) {
          acc = (acc * 1664525 + 1013904223) >>> 0;
          acc ^= k;
        }
        f.challenge_cpu_wall_ms = Math.round((performance.now() - t0c) * 1000) / 1000;
        f.challenge_cpu_acc = acc;
        if (f.pohw_triad) f.pohw_triad.cpu_wall_ms = f.challenge_cpu_wall_ms;
      } catch (eCpu) {}
      f.data_ok = !!(f.pohw_triad && f.pohw_triad.residual_mean != null);
      // Stable flat names for product matrix / scorers (aliases of richer objects)
      if (f.challenge_avg_cv != null) f.challenge_cv = f.challenge_avg_cv;
      f.pohw_triad_ok = !!f.data_ok;
      if (f.pohw_triad && f.pohw_triad.residual_mean != null) {
        f.pohw_residual_mean = f.pohw_triad.residual_mean;
        f.pohw_unique_n = f.pohw_triad.unique_n != null ? f.pohw_triad.unique_n : null;
        f.pohw_alt_changed = !!f.pohw_triad.alt_changed;
      }
      f.challenge_seed_present = !!f.has_challenge_seed;
      enqueue(ctx, "B20_challenge_seed", f, 57, "main");
    },
  });
  register("B22_gpu_timer", {
    priority: 59,
    schedule: "dynamic",
    batch_id: "B22_gpu_timer",
    layer: "hard",
    run: function (ctx) {
      var f = {
        gpu_timer_algo: "gr_h01_staircase_v6",
        pohw_direction: "H01",
        collected_at: Date.now(),
      };
      var _glRelease = null;
      function finalizeEnqueue() {
        try {
          if (_glRelease) _glRelease();
        } catch (eRel) {}
        // Product-matrix / scorer aliases for H01
        if (f.timer_query_available != null) {
          f.gpu_timer_query_available = !!f.timer_query_available;
        }
        if (f.gpu_ns_async_frames == null && f.gpu_ns_poll_frames != null) {
          f.gpu_ns_async_frames = f.gpu_ns_poll_frames;
        }
        if (f.h01_points == null && f.gpu_staircase_points != null) {
          f.h01_points = f.gpu_staircase_points;
        }
        enqueue(ctx, "B22_gpu_timer", f, 59, "main");
      }
      function fitWall(wallCurve) {
        f.gpu_staircase_points = wallCurve.length;
        f.h01_points = wallCurve.length;
        f.gpu_wall_staircase_digest = simpleHash(
          wallCurve
            .map(function (r) {
              return r.size + "x" + r.iter + ":" + Math.round((r.wall_ms || 0) * 100);
            })
            .join("|")
        ).slice(0, 16);
        if (wallCurve.length < 4) return;
        var xs = wallCurve.map(function (r) { return r.work || r.size * r.iter; });
        var ys = wallCurve.map(function (r) { return r.wall_ms; });
        var n = xs.length;
        var sx = 0, sy = 0, sxx = 0, sxy = 0;
        for (var i = 0; i < n; i++) {
          sx += xs[i]; sy += ys[i]; sxx += xs[i] * xs[i]; sxy += xs[i] * ys[i];
        }
        var den = n * sxx - sx * sx;
        f.gpu_slope_wall = den ? (n * sxy - sx * sy) / den : null;
        f.gpu_intercept_wall = den ? (sy - f.gpu_slope_wall * sx) / n : null;
        var sorted = ys.slice().sort(function (a, b) { return a - b; });
        f.gpu_wall_median_ms = sorted[Math.floor(sorted.length / 2)];
        var ssTot = 0, ssRes = 0, yMean = sy / n;
        for (var j = 0; j < n; j++) {
          var yhat = f.gpu_slope_wall * xs[j] + f.gpu_intercept_wall;
          ssTot += (ys[j] - yMean) * (ys[j] - yMean);
          ssRes += (ys[j] - yhat) * (ys[j] - yhat);
        }
        f.gpu_r2_wall = ssTot > 0 ? Math.max(0, 1 - ssRes / ssTot) : null;
      }
      function fitGpuNs(wallCurve, gpuCurve) {
        // raw meta kept only in lab (stripped on enqueue by default)
        f.gpu_query_meta = gpuCurve.slice(0, 32);
        f.gpu_ns_staircase = gpuCurve
          .filter(function (r) { return r.gpu_ns != null && r.gpu_ns > 0; })
          .map(function (r) {
            return {
              size: r.size,
              iter: r.iter,
              gpu_ns: r.gpu_ns,
              disjoint: r.disjoint,
              frames: r.frames,
            };
          });
        f.gpu_ns_staircase_digest = simpleHash(
          f.gpu_ns_staircase
            .map(function (r) {
              return r.size + "x" + r.iter + ":" + Math.round(r.gpu_ns / 1000) + (r.disjoint ? "d" : "");
            })
            .join("|")
        ).slice(0, 16);
        f.timer_query_skip =
          f.timer_query_available && f.gpu_ns_staircase.length === 0
            ? "query_result_unavailable_after_raf"
            : null;
        f.gpu_ns_readback_ok = f.gpu_ns_staircase.length > 0;
        if (!f.gpu_ns_staircase.length) {
          f.gpu_ns_median = null;
          f.gpu_disjoint_rate = f.timer_query_available ? null : 0;
          f.gpu_ns_points = 0;
          return;
        }
        var nsVals = f.gpu_ns_staircase
          .map(function (r) { return r.gpu_ns; })
          .sort(function (a, b) { return a - b; });
        f.gpu_ns_median = nsVals[Math.floor(nsVals.length / 2)];
        var dHits = f.gpu_ns_staircase.filter(function (r) { return r.disjoint; }).length;
        f.gpu_disjoint_rate = dHits / f.gpu_ns_staircase.length;
        if (f.gpu_ns_staircase.length >= 3) {
          var nxs = f.gpu_ns_staircase.map(function (r) { return r.size * r.size * r.iter; });
          var nys = f.gpu_ns_staircase.map(function (r) { return r.gpu_ns; });
          var nn = nxs.length, nsx = 0, nsy = 0, nsxx = 0, nsxy = 0;
          for (var ni = 0; ni < nn; ni++) {
            nsx += nxs[ni]; nsy += nys[ni]; nsxx += nxs[ni] * nxs[ni]; nsxy += nxs[ni] * nys[ni];
          }
          var nden = nn * nsxx - nsx * nsx;
          f.gpu_slope_ns = nden ? (nn * nsxy - nsx * nsy) / nden : null;
          // R² of GPU-ns vs work (depth beyond points count)
          if (nden && f.gpu_slope_ns != null) {
            var nIntercept = (nsy - f.gpu_slope_ns * nsx) / nn;
            var ssTotN = 0, ssResN = 0, yMeanN = nsy / nn;
            for (var nj = 0; nj < nn; nj++) {
              var yhatN = f.gpu_slope_ns * nxs[nj] + nIntercept;
              ssTotN += (nys[nj] - yMeanN) * (nys[nj] - yMeanN);
              ssResN += (nys[nj] - yhatN) * (nys[nj] - yhatN);
            }
            f.gpu_r2_ns = ssTotN > 0 ? Math.max(0, Math.min(1, 1 - ssResN / ssTotN)) : null;
          }
          // Monotonic staircase: more work → more gpu_ns (real GPU-ish)
          var monoOk = 0, monoCmp = 0;
          for (var mi = 1; mi < nn; mi++) {
            if (nxs[mi] > nxs[mi - 1]) {
              monoCmp++;
              if (nys[mi] >= nys[mi - 1] * 0.85) monoOk++;
            }
          }
          f.gpu_ns_monotonic_ratio = monoCmp ? monoOk / monoCmp : null;
        }
        // CV of gpu_ns samples (too flat or chaotic is soft-like)
        if (nsVals.length >= 3) {
          var nsMean = nsVals.reduce(function (s, x) { return s + x; }, 0) / nsVals.length;
          var nsVar = 0;
          for (var vi = 0; vi < nsVals.length; vi++) {
            var d = nsVals[vi] - nsMean;
            nsVar += d * d;
          }
          f.gpu_ns_cv = nsMean > 0 ? Math.sqrt(nsVar / nsVals.length) / nsMean : null;
        }
        var ratios = [];
        wallCurve.forEach(function (w) {
          f.gpu_ns_staircase.forEach(function (g) {
            if (g.size === w.size && g.iter === w.iter && g.gpu_ns > 0 && w.wall_ms > 0) {
              ratios.push((g.gpu_ns / 1e6) / w.wall_ms);
            }
          });
        });
        if (ratios.length) {
          ratios.sort(function (a, b) { return a - b; });
          f.gpu_ns_wall_ratio_median = ratios[Math.floor(ratios.length / 2)];
        }
        f.gpu_ns_points = f.gpu_ns_staircase.length;
        f.gpu_ns_hardware_path = true;
        f.gpu_ns_async_frames = f.gpu_ns_poll_frames || null;
        // Composite depth score 0..1 for analysis (points + r2 + monotonic + ratio sanity)
        var depth = 0;
        if (f.gpu_ns_points >= 4) depth += 0.35;
        else if (f.gpu_ns_points >= 2) depth += 0.15;
        if (f.gpu_r2_ns != null && f.gpu_r2_ns >= 0.85) depth += 0.25;
        else if (f.gpu_r2_ns != null && f.gpu_r2_ns >= 0.5) depth += 0.1;
        if (f.gpu_ns_monotonic_ratio != null && f.gpu_ns_monotonic_ratio >= 0.75) depth += 0.2;
        if (f.gpu_ns_wall_ratio_median != null && f.gpu_ns_wall_ratio_median > 0 && f.gpu_ns_wall_ratio_median < 1.2) depth += 0.2;
        if (f.gpu_ns_readback_ok) depth += 0.05;
        f.gpu_ns_depth_score = Math.min(1, depth);
      }
      try {
        var c = document.createElement("canvas");
        // Fewer size×iter combos — avoid "too many WebGL contexts" and query spam.
        var sizes = [64, 128, 256];
        var iters = [8, 16, 32];
        var wallCurve = [];
        var pendingQ = [];
        var timerAvail = false;
        // Prefer WebGL2 (native query) then WebGL1 + EXT_disjoint_timer_query
        var gl2 = null;
        try {
          gl2 = c.getContext("webgl2", { preserveDrawingBuffer: true });
        } catch (eG2) {
          gl2 = null;
        }
        var gl =
          gl2 ||
          c.getContext("webgl", { preserveDrawingBuffer: true }) ||
          c.getContext("experimental-webgl");
        if (!gl) {
          f.gpu_timer_error = "no_webgl";
          f.data_ok = false;
          finalizeEnqueue();
          return;
        }
        _glRelease = function () {
          try {
            var lose = gl.getExtension && gl.getExtension("WEBGL_lose_context");
            if (lose && lose.loseContext) /*lose_suppressed*/void 0;
          } catch (e1) {}
          try {
            c.width = 1;
            c.height = 1;
          } catch (e2) {}
        };
        // WebGL2 timer queries need EXT_disjoint_timer_query_webgl2 (not raw TIME_ELAPSED).
        // Using gl.beginQuery(0x88bf) without the extension → INVALID_ENUM spam.
        var extW2 =
          gl2 && gl2.getExtension
            ? gl2.getExtension("EXT_disjoint_timer_query_webgl2")
            : null;
        var useWebgl2Query = !!(
          gl2 &&
          extW2 &&
          typeof gl2.createQuery === "function" &&
          extW2.TIME_ELAPSED_EXT
        );
        var ext = null;
        if (!useWebgl2Query) {
          ext =
            gl.getExtension("EXT_disjoint_timer_query") ||
            gl.getExtension("EXT_disjoint_timer_query_webgl2");
        }
        if (useWebgl2Query || (ext && (ext.createQueryEXT || ext.TIME_ELAPSED_EXT))) {
          timerAvail = true;
        }
        f.timer_query_available = timerAvail;
        f.timer_query_ext = useWebgl2Query
          ? "webgl2_EXT_disjoint_timer_query"
          : timerAvail
            ? ext && ext.createQueryEXT
              ? "EXT_disjoint_timer_query"
              : "present"
            : null;
        function mk(gll, type, src) {
          var sh = gll.createShader(type);
          gll.shaderSource(sh, src);
          gll.compileShader(sh);
          return sh;
        }
        function setupQuad(gll, prog) {
          var buf = gll.createBuffer();
          gll.bindBuffer(gll.ARRAY_BUFFER, buf);
          gll.bufferData(
            gll.ARRAY_BUFFER,
            new Float32Array([-1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1]),
            gll.STATIC_DRAW
          );
          var loc = gll.getAttribLocation(prog, "p");
          gll.enableVertexAttribArray(loc);
          gll.vertexAttribPointer(loc, 2, gll.FLOAT, false, 0, 0);
        }
        sizes.forEach(function (sz) {
          c.width = sz;
          c.height = sz;
          iters.forEach(function (k) {
            var vs = mk(gl, gl.VERTEX_SHADER, "attribute vec2 p;void main(){gl_Position=vec4(p,0.0,1.0);}");
            var fs = mk(
              gl,
              gl.FRAGMENT_SHADER,
              "precision mediump float;void main(){float a=0.0;for(int i=0;i<" +
                k +
                ";i++){a+=sin(float(i)*0.17)*cos(gl_FragCoord.x*0.01+float(i));}gl_FragColor=vec4(a,a,a,1.0);}"
            );
            var prog = gl.createProgram();
            gl.attachShader(prog, vs);
            gl.attachShader(prog, fs);
            gl.linkProgram(prog);
            gl.useProgram(prog);
            setupQuad(gl, prog);
            gl.viewport(0, 0, sz, sz);
            gl.drawArrays(gl.TRIANGLES, 0, 6);
            gl.finish();
            var t0 = performance.now();
            gl.clear(gl.COLOR_BUFFER_BIT);
            gl.drawArrays(gl.TRIANGLES, 0, 6);
            gl.finish();
            var wall = performance.now() - t0;
            wallCurve.push({
              size: sz,
              iter: k,
              wall_ms: Math.round(wall * 1000) / 1000,
              work: sz * sz * k,
            });
            if (useWebgl2Query && extW2) {
              try {
                var q2 = gl.createQuery();
                var tgt2 = extW2.TIME_ELAPSED_EXT;
                gl.beginQuery(tgt2, q2);
                gl.drawArrays(gl.TRIANGLES, 0, 6);
                gl.endQuery(tgt2);
                pendingQ.push({
                  q: q2,
                  size: sz,
                  iter: k,
                  gl: gl,
                  extW2: extW2,
                  mode: "webgl2",
                });
              } catch (eQ2) {}
            } else if (ext && ext.createQueryEXT && ext.TIME_ELAPSED_EXT) {
              try {
                var q = ext.createQueryEXT();
                ext.beginQueryEXT(ext.TIME_ELAPSED_EXT, q);
                gl.drawArrays(gl.TRIANGLES, 0, 6);
                ext.endQueryEXT(ext.TIME_ELAPSED_EXT);
                pendingQ.push({ q: q, size: sz, iter: k, gl: gl, ext: ext, mode: "ext" });
              } catch (eQ) {}
            }
          });
        });
        try {
          var bw = [];
          [64, 128, 256, 512].forEach(function (tsz) {
            var tex = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D, tex);
            var pixels = new Uint8Array(tsz * tsz * 4);
            for (var i = 0; i < pixels.length; i++) pixels[i] = i & 255;
            var t1 = performance.now();
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, tsz, tsz, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
            gl.finish();
            var ms = performance.now() - t1;
            var bytes = tsz * tsz * 4;
            bw.push({
              size: tsz,
              wall_ms: Math.round(ms * 1000) / 1000,
              mib_s: ms > 0 ? Math.round((bytes / (ms / 1000) / (1024 * 1024)) * 100) / 100 : null,
            });
            gl.deleteTexture(tex);
          });
          f.gpu_bandwidth_ladder = bw;
        } catch (eBw) {
          f.gpu_bandwidth_error = String((eBw && eBw.message) || eBw);
        }
        f.gpu_wall_staircase = wallCurve;
        f.timer_query_bits = null;
        fitWall(wallCurve);
        f.data_ok = wallCurve.length >= 8;

        if (!pendingQ.length) {
          f.gpu_ns_points = 0;
          f.gpu_ns_staircase = [];
          f.gpu_ns_median = null;
          f.gpu_disjoint_rate = timerAvail ? null : 0;
          if (timerAvail) f.timer_query_skip = "no_queries_started";
          finalizeEnqueue();
          return;
        }

        var gpuCurve = [];
        var maxFrames = 90;
        var frame = 0;
        var tPoll0 = performance.now();
        function scheduleNext(fn) {
          if (typeof requestAnimationFrame === "function") {
            requestAnimationFrame(fn);
          } else {
            setTimeout(fn, 16);
          }
        }
        function readPending() {
          frame++;
          var still = [];
          pendingQ.forEach(function (item) {
            if (item.done) return;
            var avail = false;
            try {
              if (item.mode === "webgl2") {
                avail = !!item.gl.getQueryParameter(
                  item.q,
                  item.gl.QUERY_RESULT_AVAILABLE
                );
              } else {
                avail = !!item.ext.getQueryObjectEXT(
                  item.q,
                  item.ext.QUERY_RESULT_AVAILABLE_EXT
                );
              }
            } catch (eA) {
              avail = false;
              item.failed = true;
            }
            if (!avail) {
              still.push(item);
              return;
            }
            var ns = null;
            var disjoint = false;
            try {
              if (item.mode === "webgl2") {
                ns = item.gl.getQueryParameter(item.q, item.gl.QUERY_RESULT);
              } else {
                ns = item.ext.getQueryObjectEXT(item.q, item.ext.QUERY_RESULT_EXT);
              }
            } catch (eR) {
              ns = null;
            }
            try {
              if (item.mode === "webgl2") {
                // GPU_DISJOINT_EXT = 0x8FBB
                disjoint = !!item.gl.getParameter(0x8fbb);
              } else {
                disjoint = !!item.gl.getParameter(item.ext.GPU_DISJOINT_EXT);
              }
            } catch (eD) {}
            gpuCurve.push({
              size: item.size,
              iter: item.iter,
              query_started: true,
              query_available: true,
              gpu_ns: ns,
              disjoint: disjoint,
              frames: frame,
            });
            item.done = true;
            try {
              if (item.mode === "webgl2" && item.gl.deleteQuery) item.gl.deleteQuery(item.q);
              else if (item.ext && item.ext.deleteQueryEXT) item.ext.deleteQueryEXT(item.q);
            } catch (eDel) {}
          });
          pendingQ = still;
          var timedOut = frame >= maxFrames || performance.now() - tPoll0 > 2000;
          if (pendingQ.length && !timedOut) {
            try {
              gl.clear(gl.COLOR_BUFFER_BIT);
              gl.finish();
            } catch (eKick) {}
            scheduleNext(readPending);
            return;
          }
          pendingQ.forEach(function (item) {
            gpuCurve.push({
              size: item.size,
              iter: item.iter,
              query_started: true,
              query_available: false,
              gpu_ns: null,
              disjoint: false,
              frames: frame,
              timed_out: true,
            });
            try {
              if (item.mode === "webgl2" && item.gl.deleteQuery) item.gl.deleteQuery(item.q);
              else if (item.ext && item.ext.deleteQueryEXT) item.ext.deleteQueryEXT(item.q);
            } catch (eDel2) {}
          });
          f.gpu_ns_poll_frames = frame;
          f.gpu_ns_poll_ms = Math.round(performance.now() - tPoll0);
          fitGpuNs(wallCurve, gpuCurve);
          finalizeEnqueue();
        }
        scheduleNext(readPending);
        return;
      } catch (e) {
        f.gpu_timer_error = String((e && e.message) || e);
        f.data_ok = false;
        finalizeEnqueue();
      }
    },
  });
  register("B23_native_canvas_hedge", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B23_native_canvas_hedge",
    layer: "hard",
    run: function (ctx) {
      var f = {
        hedge_algo: "gr_native_canvas_hedge_v1",
        collected_at: Date.now(),
        hedge_direction: "browser_kernel+device",
      };
      function looksNative(str) {
        return typeof str === "string" && /\[native code\]/.test(str);
      }
      function safeToString(fn) {
        try {
          return Function.prototype.toString.call(fn);
        } catch (e) {
          return null;
        }
      }
      // D26-class native integrity
      try {
        var targets = {
          eval: typeof eval !== "undefined" ? eval : null,
          Function: Function,
          Object_defineProperty: Object.defineProperty,
          Array_push: Array.prototype.push,
          fetch: typeof fetch !== "undefined" ? fetch : null,
          performance_now:
            typeof performance !== "undefined" && performance.now
              ? performance.now.bind(performance)
              : null,
          canvas_toDataURL: null,
          webdriver_get: null,
        };
        try {
          if (typeof HTMLCanvasElement !== "undefined") {
            targets.canvas_toDataURL = HTMLCanvasElement.prototype.toDataURL;
          }
        } catch (e) {}
        try {
          var d = Object.getOwnPropertyDescriptor(Navigator.prototype, "webdriver");
          if (d && d.get) targets.webdriver_get = d.get;
        } catch (e) {}
        var nonNative = 0,
          checked = 0;
        var detail = {};
        Object.keys(targets).forEach(function (k) {
          var fn = targets[k];
          if (!fn) {
            detail[k] = { present: false };
            return;
          }
          checked++;
          var s = safeToString(fn);
          var nat = looksNative(s);
          if (!nat) nonNative++;
          detail[k] = { present: true, native: nat, head: s ? String(s).slice(0, 80) : null };
        });
        f.native_checked = checked;
        f.non_native_count = nonNative;
        f.native_integrity_ratio = checked ? 1 - nonNative / checked : null;
        f.native_integrity_sample = detail;
        f.native_function_toString = detail.performance_now
          ? detail.performance_now.head
          : null;
        // Automation globals hedge
        f.automation_globals = {
          webdriver: !!(navigator.webdriver),
          chrome_runtime: !!(window.chrome && window.chrome.runtime),
          callPhantom: !!(window.callPhantom || window._phantom),
          __selenium: !!(window.__selenium_unwrapped || window._Selenium_IDE_Recorder),
          __webdriver: !!(window.__webdriver_evaluate || window.__driver_evaluate),
        };
      } catch (eN) {
        f.native_error = String((eN && eN.message) || eN);
      }
      // D02-class canvas geometry digest
      try {
        var c = document.createElement("canvas");
        c.width = 220;
        c.height = 140;
        var g = c.getContext("2d", { willReadFrequently: true });
        if (g) {
          g.fillStyle = "#f0e6d2";
          g.fillRect(0, 0, 220, 140);
          g.fillStyle = "rgb(42, 110, 200)";
          g.fillRect(12, 18, 80, 55);
          g.strokeStyle = "rgb(200, 40, 40)";
          g.lineWidth = 2.5;
          g.beginPath();
          g.arc(150, 70, 38, 0.15, Math.PI * 1.7);
          g.stroke();
          var img = g.getImageData(0, 0, 220, 140);
          var d = img.data;
          var sum = 0,
            n = 0;
          for (var i = 0; i < d.length; i += 16) {
            sum += d[i];
            n++;
          }
          f.canvas_geometry_mean = Math.round((sum / n / 255) * 1e8) / 1e8;
          f.canvas_geometry_hash = simpleHash(
            String(d[0]) + "," + d[100] + "," + d[500] + "," + d[2000] + "|" + f.canvas_geometry_mean
          ).slice(0, 16);
          f.canvas_hash = f.canvas_geometry_hash;
          // stability: second run
          var g2 = c.getContext("2d");
          var img2 = g2.getImageData(0, 0, 220, 140);
          f.canvas_geometry_stable =
            img2.data[0] === d[0] && img2.data[100] === d[100] && img2.data[500] === d[500];
        }
      } catch (eC) {
        f.canvas_hedge_error = String((eC && eC.message) || eC);
      }
      // D25-class math + wasm-ish timing
      try {
        var acc = 0;
        for (var k = 0; k < 2000; k++) {
          acc += Math.sin(k * 0.017) * Math.cos(k * 0.013) + Math.tan(k % 50) * 1e-6;
        }
        f.math_digest = "m_" + simpleHash(String(acc)).slice(0, 12);
        f.math_acc = Math.round(acc * 1e6) / 1e6;
        var t0 = performance.now();
        var w = 0;
        for (var j = 0; j < 8000; j++) w = (w * 31 + j) >>> 0;
        f.wasm_timing_ms = Math.round((performance.now() - t0) * 1000) / 1000;
        f.wasm_acc = w;
      } catch (eM) {}
      f.data_ok =
        f.native_integrity_ratio != null ||
        !!f.canvas_geometry_hash ||
        !!f.math_digest;
      f.hedge_material_count =
        (f.native_integrity_ratio != null ? 1 : 0) +
        (f.canvas_geometry_hash ? 1 : 0) +
        (f.math_digest ? 1 : 0);
      enqueue(ctx, "B23_native_canvas_hedge", f, 55, "main");
    },
  });
  register("B31_shader_numeric", {
    priority: 58,
    schedule: "dynamic",
    batch_id: "B31_shader_numeric",
    layer: "hard",
    run: function (ctx) {
      var f = {
        shader_numeric_algo: "gr_h03_shader_numeric_v2",
        pohw_direction: "H03",
        collected_at: Date.now(),
      };
      try {
        var c = document.createElement("canvas");
        c.width = 32;
        c.height = 32;
        var gl =
          c.getContext("webgl", { preserveDrawingBuffer: true }) ||
          c.getContext("experimental-webgl");
        if (!gl) {
          f.shader_skip = "no_webgl";
          f.data_ok = false;
          enqueue(ctx, "B31_shader_numeric", f, 58, "main");
          return;
        }
        var stages = [gl.VERTEX_SHADER, gl.FRAGMENT_SHADER];
        var types = [
          gl.LOW_FLOAT,
          gl.MEDIUM_FLOAT,
          gl.HIGH_FLOAT,
          gl.LOW_INT,
          gl.MEDIUM_INT,
          gl.HIGH_INT,
        ];
        var matrix = [];
        stages.forEach(function (st) {
          types.forEach(function (tp) {
            try {
              var p = gl.getShaderPrecisionFormat(st, tp);
              matrix.push(p ? [p.rangeMin, p.rangeMax, p.precision] : null);
            } catch (e) {
              matrix.push(null);
            }
          });
        });
        f.gl_precision_matrix = matrix;
        f.precision_matrix = matrix;
        f.precision_digest = simpleHash(JSON.stringify(matrix)).slice(0, 12);
        // highp float precision claim
        try {
          var hp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
          f.highp_precision = hp ? hp.precision : null;
          var mp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.MEDIUM_FLOAT);
          f.mediump_precision = mp ? mp.precision : null;
        } catch (eH) {}
        // multi-iter mediump vs highp: ULP-like spectrum
        function drawPrec(prec, iters) {
          function mk(type, src) {
            var sh = gl.createShader(type);
            gl.shaderSource(sh, src);
            gl.compileShader(sh);
            return sh;
          }
          var vs = mk(
            gl.VERTEX_SHADER,
            "attribute vec2 p;void main(){gl_Position=vec4(p,0.0,1.0);}"
          );
          var fs = mk(
            gl.FRAGMENT_SHADER,
            prec +
              " float;void main(){float x=1.0;for(int i=0;i<" +
              iters +
              ";i++){x=sin(x*1.0001+0.17+float(i)*0.0003);}gl_FragColor=vec4(fract(x),fract(x*1.7),fract(x*2.3),1.0);}"
          );
          var prog = gl.createProgram();
          gl.attachShader(prog, vs);
          gl.attachShader(prog, fs);
          gl.linkProgram(prog);
          gl.useProgram(prog);
          var buf = gl.createBuffer();
          gl.bindBuffer(gl.ARRAY_BUFFER, buf);
          gl.bufferData(
            gl.ARRAY_BUFFER,
            new Float32Array([-1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1]),
            gl.STATIC_DRAW
          );
          var loc = gl.getAttribLocation(prog, "p");
          gl.enableVertexAttribArray(loc);
          gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
          gl.viewport(0, 0, 32, 32);
          gl.drawArrays(gl.TRIANGLES, 0, 6);
          var pix = new Uint8Array(32 * 32 * 4);
          gl.readPixels(0, 0, 32, 32, gl.RGBA, gl.UNSIGNED_BYTE, pix);
          var sum = 0;
          for (var i = 0; i < pix.length; i += 4) sum += pix[i];
          return sum / (pix.length / 4);
        }
        try {
          var itersList = [8, 16, 32, 64];
          var ulpPoints = [];
          var maxUlp = 0;
          var sumUlp = 0;
          itersList.forEach(function (it) {
            var hi = drawPrec("precision highp", it);
            var med = drawPrec("precision mediump", it);
            var d = Math.abs(hi - med);
            var ulp = Math.min(64, d);
            ulpPoints.push({
              iters: it,
              highp: Math.round(hi * 1e4) / 1e4,
              mediump: Math.round(med * 1e4) / 1e4,
              delta: Math.round(d * 1e4) / 1e4,
            });
            if (ulp > maxUlp) maxUlp = ulp;
            sumUlp += ulp;
          });
          f.highp_mean = ulpPoints.length ? ulpPoints[ulpPoints.length - 1].highp : null;
          f.mediump_mean = ulpPoints.length ? ulpPoints[ulpPoints.length - 1].mediump : null;
          f.mediump_diverges = maxUlp > 0.5;
          f.shader_ulp_max = Math.round(maxUlp * 1000) / 1000;
          f.shader_ulp_mean =
            ulpPoints.length > 0
              ? Math.round((sumUlp / ulpPoints.length) * 1000) / 1000
              : null;
          // Spectrum digest (not raw pixels)
          f.shader_ulp_spectrum = simpleHash(
            ulpPoints
              .map(function (p) {
                return p.iters + ":" + p.delta;
              })
              .join("|")
          ).slice(0, 16);
          f.shader_ulp_points_n = ulpPoints.length;
        } catch (eD) {
          f.shader_draw_error = String((eD && eD.message) || eD);
        }
        f.data_ok = matrix.length >= 12;
      } catch (e) {
        f.shader_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B31_shader_numeric", f, 58, "main");
    },
  });
  register("B33_caps_pressure", {
    priority: 56,
    schedule: "dynamic",
    batch_id: "B33_caps_pressure",
    layer: "hard",
    run: function (ctx) {
      var f = {
        caps_algo: "gr_h05_caps_pressure_v2",
        pohw_direction: "H05",
        collected_at: Date.now(),
      };
      try {
        var c = document.createElement("canvas");
        c.width = 4;
        c.height = 4;
        var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
        if (!gl) {
          f.caps_skip = "no_webgl";
          f.data_ok = false;
          enqueue(ctx, "B33_caps_pressure", f, 56, "main");
          return;
        }
        var claimed = gl.getParameter(gl.MAX_TEXTURE_SIZE) || 0;
        f.claimed_max_tex = claimed;
        f.gl_max_texture_size = claimed;
        f.gl_max_renderbuffer = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE);
        f.gl_max_cube_map = gl.getParameter(gl.MAX_CUBE_MAP_TEXTURE_SIZE);
        f.gl_max_vertex_attribs = gl.getParameter(gl.MAX_VERTEX_ATTRIBS);
        // Probe actual alloc sizes until failure (bounded for UX)
        // denser ladder to catch soft stacks that pass only power-of-two claims
        var sizes = [128, 256, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144, 8192];
        if (claimed >= 16384) sizes.push(12288, 16384);
        // Never try full 32768 in page (memory bomb)
        var actual = 0;
        var failed_at = null;
        var results = [];
        var okCount = 0;
        for (var i = 0; i < sizes.length; i++) {
          var sz = sizes[i];
          if (claimed > 0 && sz > claimed) break;
          try {
            var tex = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D, tex);
            // empty texImage2D with null data — still validates size limits
            gl.texImage2D(
              gl.TEXTURE_2D,
              0,
              gl.RGBA,
              sz,
              sz,
              0,
              gl.RGBA,
              gl.UNSIGNED_BYTE,
              null
            );
            var err = gl.getError();
            var ok = err === gl.NO_ERROR;
            results.push({ size: sz, ok: ok, err: err });
            if (ok) {
              actual = sz;
              okCount++;
            } else {
              failed_at = sz;
              gl.deleteTexture(tex);
              break;
            }
            gl.deleteTexture(tex);
          } catch (eA) {
            failed_at = sz;
            results.push({ size: sz, ok: false, err: String(eA) });
            break;
          }
        }
        // Renderbuffer pressure (independent fail point)
        var rbClaimed = f.gl_max_renderbuffer || 0;
        var rbSizes = [256, 512, 1024, 2048, 4096, 8192];
        if (rbClaimed >= 16384) rbSizes.push(16384);
        var rbActual = 0;
        var rbFail = null;
        for (var ri = 0; ri < rbSizes.length; ri++) {
          var rsz = rbSizes[ri];
          if (rbClaimed > 0 && rsz > rbClaimed) break;
          try {
            var rb = gl.createRenderbuffer();
            gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
            gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA4, rsz, rsz);
            var rerr = gl.getError();
            if (rerr === gl.NO_ERROR) rbActual = rsz;
            else {
              rbFail = rsz;
              gl.deleteRenderbuffer(rb);
              break;
            }
            gl.deleteRenderbuffer(rb);
          } catch (eRb) {
            rbFail = rsz;
            break;
          }
        }
        f.actual_max_tex = actual;
        f.caps_actual_max_tex = actual; // product matrix / scorer name
        f.caps_claimed_max_tex = claimed;
        f.caps_failed_at = failed_at;
        f.caps_alloc_fail_at = failed_at; // H05 alias
        f.caps_probe_n = results.length;
        f.caps_ok_n = okCount;
        f.caps_rb_actual = rbActual;
        f.caps_rb_fail_at = rbFail;
        // lab-only raw results (stripped on enqueue by default)
        f.caps_probe_results = results;
        f.caps_claim_vs_actual_gap =
          claimed && actual ? Math.max(0, claimed - actual) : null;
        f.caps_claim_vs_actual =
          claimed >= 16384 && actual > 0 && actual < claimed * 0.5;
        // Internal consistency: huge texture claim with small renderbuffer is suspicious
        var rbMax = f.gl_max_renderbuffer || 0;
        f.caps_internal_inconsistent =
          claimed >= 16384 && rbMax > 0 && rbMax < 8192;
        f.caps_rb_vs_claim =
          rbClaimed && rbActual ? Math.max(0, rbClaimed - rbActual) : null;
        f.data_ok = actual > 0 || results.length > 0;
      } catch (e) {
        f.caps_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B33_caps_pressure", f, 56, "main");
    },
  });
  register("B30_gpu_bandwidth",
      "B84_gpu_bandwidth_ladder", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B30_gpu_bandwidth",
    layer: "hard",
    run: function (ctx) {
      var f = {
        bw_algo: "gr_h02_bandwidth_v1",
        pohw_direction: "H02",
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
          enqueue(ctx, "B30_gpu_bandwidth", f, 55, "main");
          return;
        }
        var sizes = [64, 128, 256, 512, 1024, 2048]; // iss/60 L5 A4
        var upload = [];
        var readback = [];
        var roundtrip = [];
        sizes.forEach(function (sz) {
          c.width = sz;
          c.height = sz;
          var pixels = new Uint8Array(sz * sz * 4);
          for (var i = 0; i < pixels.length; i++) pixels[i] = (i * 17) & 255;
          var tex = gl.createTexture();
          gl.bindTexture(gl.TEXTURE_2D, tex);
          var t0 = performance.now();
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, sz, sz, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
          gl.finish();
          var upMs = performance.now() - t0;
          var bytes = sz * sz * 4;
          upload.push({
            size: sz,
            wall_ms: Math.round(upMs * 1000) / 1000,
            mib_s: upMs > 0 ? Math.round((bytes / (upMs / 1000) / (1024 * 1024)) * 100) / 100 : null,
          });
          // draw + readback
          gl.viewport(0, 0, sz, sz);
          gl.clearColor(0.1, 0.2, 0.3, 1);
          gl.clear(gl.COLOR_BUFFER_BIT);
          var out = new Uint8Array(sz * sz * 4);
          var t1 = performance.now();
          gl.readPixels(0, 0, sz, sz, gl.RGBA, gl.UNSIGNED_BYTE, out);
          var rbMs = performance.now() - t1;
          readback.push({
            size: sz,
            wall_ms: Math.round(rbMs * 1000) / 1000,
            mib_s: rbMs > 0 ? Math.round((bytes / (rbMs / 1000) / (1024 * 1024)) * 100) / 100 : null,
          });
          // roundtrip: upload+draw+read
          var t2 = performance.now();
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, sz, sz, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
          gl.clear(gl.COLOR_BUFFER_BIT);
          gl.readPixels(0, 0, Math.min(sz, 64), Math.min(sz, 64), gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(64 * 64 * 4));
          gl.finish();
          roundtrip.push({ size: sz, wall_ms: Math.round((performance.now() - t2) * 1000) / 1000 });
          gl.deleteTexture(tex);
        });
        f.gpu_bandwidth_ladder = upload;
        f.gpu_readback_ladder = readback;
        f.gpu_roundtrip_ladder = roundtrip;
        // intercept of roundtrip vs size (fixed sync cost)
        if (roundtrip.length >= 3) {
          var xs = roundtrip.map(function (r) { return r.size; });
          var ys = roundtrip.map(function (r) { return r.wall_ms; });
          var n = xs.length, sx = 0, sy = 0, sxx = 0, sxy = 0;
          for (var i = 0; i < n; i++) {
            sx += xs[i]; sy += ys[i]; sxx += xs[i] * xs[i]; sxy += xs[i] * ys[i];
          }
          var den = n * sxx - sx * sx;
          f.roundtrip_slope = den ? (n * sxy - sx * sy) / den : null;
          f.roundtrip_intercept_ms = den ? (sy - f.roundtrip_slope * sx) / n : null;
        }
        f.data_ok = upload.length >= 3;
      } catch (e) {
        f.bw_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B30_gpu_bandwidth", f, 55, "main");
    },
  });
  register("B34_cpu_cache_ladder", {
    priority: 52,
    schedule: "dynamic",
    batch_id: "B34_cpu_cache_ladder",
    layer: "hard",
    run: function (ctx) {
      var f = {
        cpu_cache_algo: "gr_h07_cache_ladder_v1",
        pohw_direction: "H07",
        collected_at: Date.now(),
      };
      try {
        var sizes = [4096, 16384, 65536, 262144, 1048576, 4194304];
        var ladder = [];
        sizes.forEach(function (bytes) {
          var n = (bytes / 4) | 0;
          var buf = new Int32Array(n);
          for (var i = 0; i < n; i++) buf[i] = i;
          var t0 = performance.now();
          var acc = 0;
          var step = 16;
          for (var r = 0; r < 4; r++) {
            for (var j = 0; j < n; j += step) acc = (acc + buf[j]) | 0;
          }
          var ms = performance.now() - t0;
          ladder.push({
            bytes: bytes,
            wall_ms: Math.round(ms * 1000) / 1000,
            acc: acc,
            ns_per_touch: n > 0 ? (ms * 1e6) / (n * 4 / step) : null,
          });
        });
        f.cpu_cache_ladder = ladder;
        // knee: first size where ns_per_touch jumps >1.8x previous
        var knee = null;
        for (var k = 1; k < ladder.length; k++) {
          var prev = ladder[k - 1].ns_per_touch || 1;
          var cur = ladder[k].ns_per_touch || 1;
          if (cur / prev > 1.8) {
            knee = ladder[k].bytes;
            break;
          }
        }
        f.cpu_cache_knee_bytes = knee;
        f.data_ok = ladder.length >= 4;
      } catch (e) {
        f.cpu_cache_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B34_cpu_cache_ladder", f, 52, "main");
    },
  });
  register("B36_raster_msaa", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B36_raster_msaa",
    layer: "hard",
    run: function (ctx) {
      var f = {
        raster_algo: "gr_h04_raster_msaa_v1",
        pohw_direction: "H04",
        collected_at: Date.now(),
      };
      try {
        var c = document.createElement("canvas");
        c.width = 64;
        c.height = 64;
        var gl =
          c.getContext("webgl", { antialias: true, preserveDrawingBuffer: true }) ||
          c.getContext("experimental-webgl", { antialias: true, preserveDrawingBuffer: true });
        if (!gl) {
          f.raster_skip = "no_webgl";
          f.data_ok = false;
          enqueue(ctx, "B36_raster_msaa", f, 54, "main");
          return;
        }
        f.samples = gl.getParameter(gl.SAMPLES);
        f.sample_buffers = gl.getParameter(gl.SAMPLE_BUFFERS);
        f.aliased_line_width_range = gl.getParameter(gl.ALIASED_LINE_WIDTH_RANGE);
        f.aliased_point_size_range = gl.getParameter(gl.ALIASED_POINT_SIZE_RANGE);
        // Edge rule: thin triangle coverage at pixel centers
        function drawTri(ox, oy) {
          function mk(type, src) {
            var sh = gl.createShader(type);
            gl.shaderSource(sh, src);
            gl.compileShader(sh);
            return sh;
          }
          var vs = mk(
            gl.VERTEX_SHADER,
            "attribute vec2 p;void main(){gl_Position=vec4(p,0.0,1.0);}"
          );
          var fs = mk(
            gl.FRAGMENT_SHADER,
            "precision mediump float;void main(){gl_FragColor=vec4(1.0,0.0,0.0,1.0);}"
          );
          var prog = gl.createProgram();
          gl.attachShader(prog, vs);
          gl.attachShader(prog, fs);
          gl.linkProgram(prog);
          gl.useProgram(prog);
          var buf = gl.createBuffer();
          gl.bindBuffer(gl.ARRAY_BUFFER, buf);
          // tiny triangle around center with subpixel offset
          var cx = ox * 0.02;
          var cy = oy * 0.02;
          gl.bufferData(
            gl.ARRAY_BUFFER,
            new Float32Array([
              -0.05 + cx, -0.05 + cy,
              0.05 + cx, -0.05 + cy,
              0.0 + cx, 0.05 + cy,
            ]),
            gl.STATIC_DRAW
          );
          var loc = gl.getAttribLocation(prog, "p");
          gl.enableVertexAttribArray(loc);
          gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
          gl.viewport(0, 0, 64, 64);
          gl.clearColor(0, 0, 0, 1);
          gl.clear(gl.COLOR_BUFFER_BIT);
          gl.drawArrays(gl.TRIANGLES, 0, 3);
          var pix = new Uint8Array(64 * 64 * 4);
          gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, pix);
          var sum = 0, n = 0;
          for (var i = 0; i < pix.length; i += 4) {
            sum += pix[i];
            n++;
          }
          return Math.round((sum / n) * 1000) / 1000;
        }
        var offsets = [0, 0.25, 0.5, 0.75, 1.0];
        var curve = offsets.map(function (o) {
          return { o: o, mean: drawTri(o, o) };
        });
        f.raster_edge_curve = curve;
        f.raster_edge_hash = simpleHash(
          curve.map(function (r) { return r.mean; }).join(",")
        ).slice(0, 12);
        // dither/sRGB residual lite
        try {
          gl.enable(gl.DITHER);
          f.dither_enabled = true;
        } catch (eD) {
          f.dither_enabled = false;
        }
        f.data_ok = curve.length >= 3;
      } catch (e) {
        f.raster_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B36_raster_msaa", f, 54, "main");
    },
  });
  register("B37_thermal_drift_lite", {
    priority: 44,
    schedule: "dynamic",
    batch_id: "B37_thermal_drift_lite",
    layer: "hard",
    run: function (ctx) {
      var f = {
        thermal_algo: "gr_h09_thermal_lite_v1",
        pohw_direction: "H09",
        collected_at: Date.now(),
      };
      try {
        var bursts = [];
        // 6 short CPU+GPU bursts
        for (var b = 0; b < 6; b++) {
          var t0 = performance.now();
          var acc = 0;
          for (var i = 0; i < 80000; i++) acc += Math.sin(i * 0.013) * Math.cos(i * 0.017);
          var cpuMs = performance.now() - t0;
          var gpuMs = null;
          try {
            var c = document.createElement("canvas");
            c.width = 128;
            c.height = 128;
            var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
            if (gl) {
              var t1 = performance.now();
              for (var g = 0; g < 30; g++) {
                gl.clearColor((g % 10) / 10, 0.2, 0.3, 1);
                gl.clear(gl.COLOR_BUFFER_BIT);
              }
              gl.finish();
              gpuMs = performance.now() - t1;
            }
          } catch (eG) {}
          bursts.push({
            i: b,
            cpu_ms: Math.round(cpuMs * 1000) / 1000,
            gpu_ms: gpuMs != null ? Math.round(gpuMs * 1000) / 1000 : null,
            acc: acc,
          });
        }
        f.thermal_bursts = bursts;
        var cpus = bursts.map(function (x) { return x.cpu_ms; });
        var first = cpus[0] || 1;
        var last = cpus[cpus.length - 1] || 1;
        f.thermal_cpu_slope = Math.round(((last - first) / first) * 10000) / 10000;
        f.thermal_cpu_cv = (function () {
          var m = cpus.reduce(function (s, x) { return s + x; }, 0) / cpus.length;
          var v = 0;
          cpus.forEach(function (x) { v += (x - m) * (x - m); });
          return m > 0 ? Math.sqrt(v / cpus.length) / m : null;
        })();
        f.data_ok = bursts.length >= 4;
      } catch (e) {
        f.thermal_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B37_thermal_drift_lite", f, 44, "main");
    },
  });
  register("B42_thermal_drift_full", {
    priority: 38,
    schedule: "dynamic",
    batch_id: "B42_thermal_drift_full",
    layer: "hard",
    run: function (ctx) {
      var f = {
        thermal_algo: "gr_h09_thermal_full_v2",
        pohw_direction: "H09",
        collected_at: Date.now(),
        thermal_mode: "full_progressive",
        thermal_progressive: true,
      };
      var maxMs = (ctx && ctx.thermal_full_max_ms) || (global.__GR_THERMAL_FULL_MS__) || 45000;
      maxMs = Math.min(Math.max(Number(maxMs) || 45000, 8000), 120000);
      f.thermal_budget_ms = maxMs;
      var phases = [
        { name: "A", bursts: 8, work: 60000, min_budget_frac: 0.0 },
        { name: "B", bursts: 12, work: 100000, min_budget_frac: 0.2 },
        { name: "C", bursts: 16, work: 140000, min_budget_frac: 0.45 },
      ];
      // optional phase D when budget >= 60s (true long probe)
      if (maxMs >= 60000) {
        phases.push({ name: "D", bursts: 20, work: 180000, min_budget_frac: 0.55 });
      }
      if (maxMs >= 90000) {
        phases.push({ name: "E", bursts: 24, work: 200000, min_budget_frac: 0.7 });
      }
      var tStart = performance.now();
      var all = [];
      var phaseIdx = 0;

      function cpuBurst(work) {
        var t0 = performance.now();
        var acc = 0;
        for (var i = 0; i < work; i++) acc += Math.sin(i * 0.011) * Math.cos(i * 0.019);
        return { cpu_ms: performance.now() - t0, acc: acc };
      }
      function gpuBurst() {
        try {
          var c = document.createElement("canvas");
          c.width = 256;
          c.height = 256;
          var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
          if (!gl) return null;
          var t0 = performance.now();
          for (var g = 0; g < 40; g++) {
            gl.clearColor((g % 10) / 10, 0.15, 0.25, 1);
            gl.clear(gl.COLOR_BUFFER_BIT);
          }
          gl.finish();
          return performance.now() - t0;
        } catch (e) {
          return null;
        }
      }
      function summarize() {
        f.thermal_bursts_full = all;
        f.thermal_elapsed_ms = Math.round(performance.now() - tStart);
        f.thermal_phases_done = phaseIdx;
        if (all.length >= 4) {
          var cpus = all.map(function (x) { return x.cpu_ms; });
          var m = cpus.reduce(function (s, x) { return s + x; }, 0) / cpus.length;
          var v = 0;
          cpus.forEach(function (x) { v += (x - m) * (x - m); });
          f.thermal_cpu_cv_full = m > 0 ? Math.sqrt(v / cpus.length) / m : null;
          f.thermal_cpu_slope_full =
            Math.round(((cpus[cpus.length - 1] - cpus[0]) / (cpus[0] || 1)) * 10000) / 10000;
          var gpus = all.map(function (x) { return x.gpu_ms; }).filter(function (x) { return x != null; });
          if (gpus.length >= 4) {
            f.thermal_gpu_slope_full =
              Math.round(((gpus[gpus.length - 1] - gpus[0]) / (gpus[0] || 1)) * 10000) / 10000;
          }
        }
        f.data_ok = all.length >= 8;
        enqueue(ctx, "B42_thermal_drift_full", f, 38, "main");
      }
      function runPhase() {
        if (phaseIdx >= phases.length) {
          summarize();
          return;
        }
        var elapsed = performance.now() - tStart;
        if (elapsed > maxMs * 0.98) {
          f.thermal_stopped = "budget";
          summarize();
          return;
        }
        var ph = phases[phaseIdx];
        // only start phase if remaining budget allows meaningful work
        if (elapsed < maxMs * ph.min_budget_frac) {
          // still early enough — ok
        }
        if (elapsed > maxMs * 0.95 && phaseIdx > 0) {
          f.thermal_stopped = "budget_before_" + ph.name;
          summarize();
          return;
        }
        var series = [];
        for (var b = 0; b < ph.bursts; b++) {
          if (performance.now() - tStart > maxMs) break;
          var cb = cpuBurst(ph.work);
          var gb = gpuBurst();
          series.push({
            phase: ph.name,
            i: b,
            cpu_ms: Math.round(cb.cpu_ms * 1000) / 1000,
            gpu_ms: gb != null ? Math.round(gb * 1000) / 1000 : null,
            t_rel_ms: Math.round(performance.now() - tStart),
          });
        }
        all = all.concat(series);
        f["thermal_phase_" + ph.name + "_n"] = series.length;
        if (series.length >= 3) {
          var first = series[0].cpu_ms || 1;
          var last = series[series.length - 1].cpu_ms || 1;
          f["thermal_phase_" + ph.name + "_slope"] =
            Math.round(((last - first) / first) * 10000) / 10000;
        }
        phaseIdx++;
        // yield to event loop between phases (progressive, non-blocking long probe)
        setTimeout(runPhase, 30);
      }
      // start after microtask so lite packs can finish first
      setTimeout(runPhase, 0);
    },
  });
  register("B46_audio_deep", {
    priority: 35,
    schedule: "dynamic",
    batch_id: "B46_audio_deep",
    layer: "hard",
    run: function (ctx) {
      var f = {
        audio_deep_algo: "gr_d03_audio_deep_v2_multiseed",
        pohw_direction: "D03",
        collected_at: Date.now(),
      };
      function round6(x) {
        return Math.round(x * 1e6) / 1e6;
      }
      function moments(samples) {
        var n = samples.length;
        if (!n) return { n: 0 };
        var sum = 0, sum2 = 0, min = Infinity, max = -Infinity;
        for (var i = 0; i < n; i++) {
          var x = samples[i];
          sum += x;
          sum2 += x * x;
          if (x < min) min = x;
          if (x > max) max = x;
        }
        var mean = sum / n;
        var variance = sum2 / n - mean * mean;
        var sum3 = 0, sum4 = 0;
        for (var j = 0; j < n; j++) {
          var d = samples[j] - mean;
          var d2 = d * d;
          sum3 += d2 * d;
          sum4 += d2 * d2;
        }
        var std = Math.sqrt(Math.max(variance, 0));
        return {
          n: n,
          mean: round6(mean),
          variance: round6(variance),
          skew: std > 0 ? round6(sum3 / n / (std * std * std)) : 0,
          kurtosis: std > 0 ? round6(sum4 / n / (std * std * std * std) - 3) : 0,
          min: round6(min),
          max: round6(max),
          rms: round6(Math.sqrt(sum2 / n)),
        };
      }
      function peakBins(samples, bins) {
        var hist = [];
        for (var b = 0; b < bins; b++) hist.push(0);
        var win = Math.floor(samples.length / 64) || 1;
        for (var w = 0; w < 64; w++) {
          var peak = 0;
          var start = w * win;
          var end = Math.min(samples.length, start + win);
          for (var i = start; i < end; i++) {
            var a = Math.abs(samples[i]);
            if (a > peak) peak = a;
          }
          hist[Math.min(bins - 1, Math.floor(peak * bins))]++;
        }
        var tot = hist.reduce(function (s, x) { return s + x; }, 0) || 1;
        return hist.map(function (h) { return Math.round((h / tot) * 1e5) / 1e5; });
      }
      function subsampleCurve(samples, n) {
        var step = Math.max(1, Math.floor(samples.length / n));
        var curve = [];
        for (var i = 0; i < samples.length && curve.length < n; i += step) {
          curve.push(round6(samples[i]));
        }
        return curve;
      }
      /** Zero-crossing / sign-run phase digest between two seeded renders. */
      function phaseDigest(a, b) {
        var n = Math.min(a.length, b.length);
        if (n < 64) return null;
        var step = Math.max(1, Math.floor(n / 64));
        var xor = 0, agree = 0, slots = 0, lagBest = 0, lagScore = -1;
        var lags = [-2, -1, 0, 1, 2];
        for (var li = 0; li < lags.length; li++) {
          var lag = lags[li];
          var score = 0, cnt = 0;
          for (var i = 0; i < n; i += step) {
            var j = i + lag;
            if (j < 0 || j >= n) continue;
            var sa = a[i] >= 0 ? 1 : 0;
            var sb = b[j] >= 0 ? 1 : 0;
            score += sa === sb ? 1 : 0;
            cnt++;
          }
          if (cnt && score / cnt > lagScore) {
            lagScore = score / cnt;
            lagBest = lag;
          }
        }
        for (var k = 0; k < n; k += step) {
          var aa = a[k] >= 0 ? 1 : 0;
          var bb = b[k] >= 0 ? 1 : 0;
          if (aa === bb) agree++;
          else xor++;
          slots++;
        }
        return {
          sign_agree: slots ? round6(agree / slots) : null,
          sign_xor: slots ? round6(xor / slots) : null,
          best_lag: lagBest,
          best_lag_agree: lagScore >= 0 ? round6(lagScore) : null,
          slots: slots,
        };
      }
      function renderSeed(Offline, sampleRate, length, seed) {
        var actx = new Offline(1, length, sampleRate);
        var osc = actx.createOscillator();
        osc.type = seed.t1 || "triangle";
        osc.frequency.value = seed.f1;
        var osc2 = actx.createOscillator();
        osc2.type = seed.t2 || "sawtooth";
        osc2.frequency.value = seed.f2;
        var gain = actx.createGain();
        gain.gain.value = 0.35;
        var comp = actx.createDynamicsCompressor();
        comp.threshold.setValueAtTime(-42, 0);
        comp.knee.setValueAtTime(28, 0);
        comp.ratio.setValueAtTime(14, 0);
        comp.attack.setValueAtTime(0.004, 0);
        comp.release.setValueAtTime(0.18, 0);
        var biquad = actx.createBiquadFilter();
        biquad.type = "lowpass";
        biquad.frequency.value = seed.lp || 3500;
        biquad.Q.value = seed.q || 1.2;
        osc.connect(gain);
        osc2.connect(gain);
        gain.connect(biquad);
        biquad.connect(comp);
        comp.connect(actx.destination);
        osc.start(0);
        osc2.start(0);
        try {
          osc.frequency.exponentialRampToValueAtTime(seed.f1 * 2.2, length / sampleRate);
        } catch (eR) {
          try {
            osc.frequency.linearRampToValueAtTime(seed.f1 * 2.2, length / sampleRate);
          } catch (eR2) {}
        }
        osc.stop(length / sampleRate);
        osc2.stop(length / sampleRate);
        return Promise.resolve(actx.startRendering()).then(function (buf) {
          var ch = buf.getChannelData(0);
          var samples = new Float32Array(ch.length);
          samples.set(ch);
          return { samples: samples, sr: buf.sampleRate };
        });
      }
      /**
       * iss/54 P7 — ConvolverNode impulse response + IIR near-pole quant probe.
       * Serial after dual-seed (same audio resource class). FFT block size / accumulate
       * order is implementation-dependent → silicon/stack micro-diff.
       */
      function renderConvolverIir(Offline, sampleRate, length) {
        var actx = new Offline(1, length, sampleRate);
        // Deterministic impulse IR (not random — stable fingerprint)
        var irLen = 256;
        var irBuf = actx.createBuffer(1, irLen, sampleRate);
        var ir = irBuf.getChannelData(0);
        var ii;
        for (ii = 0; ii < irLen; ii++) {
          // decaying sinc-like impulse — stack FP sensitive
          var t = ii / sampleRate;
          ir[ii] = Math.exp(-t * 1200) * Math.sin(2 * Math.PI * 440 * t + ii * 0.017);
        }
        var conv = actx.createConvolver();
        try {
          conv.normalize = true;
        } catch (eN) {}
        conv.buffer = irBuf;
        var osc = actx.createOscillator();
        osc.type = "sawtooth";
        osc.frequency.value = 880;
        var iir = null;
        try {
          // poles near unit circle — coefficient quant differs by stack
          iir = actx.createIIRFilter(
            [0.1, 0.2, 0.1],
            [1.0, -1.7, 0.8]
          );
        } catch (eIir) {
          iir = null;
        }
        var gain = actx.createGain();
        gain.gain.value = 0.25;
        osc.connect(gain);
        if (iir) {
          gain.connect(iir);
          iir.connect(conv);
        } else {
          gain.connect(conv);
        }
        conv.connect(actx.destination);
        osc.start(0);
        osc.stop(length / sampleRate);
        return Promise.resolve(actx.startRendering()).then(function (buf) {
          var ch = buf.getChannelData(0);
          var samples = new Float32Array(ch.length);
          samples.set(ch);
          return samples;
        });
      }
      function finishDual(a, b, sr, convSamples) {
        var m = moments(a);
        f.audio_deep_moments = m;
        f.audio_deep_peak_bins = peakBins(a, 16);
        f.audio_deep_sr = sr;
        f.audio_deep_curve = subsampleCurve(a, 48);
        f.audio_deep_curve_b = subsampleCurve(b, 48);
        f.audio_deep_seeds = 2;
        f.audio_deep_phase = phaseDigest(a, b);
        f.audio_deep_phase_digest = f.audio_deep_phase
          ? simpleHash(
              [
                f.audio_deep_phase.sign_agree,
                f.audio_deep_phase.best_lag,
                f.audio_deep_phase.best_lag_agree,
                m.mean,
                m.rms,
              ].join("|")
            ).slice(0, 12)
          : null;
        if (convSamples && convSamples.length > 100) {
          f.audio_convolver_curve = subsampleCurve(convSamples, 48);
          f.audio_convolver_moments = moments(convSamples);
          f.audio_convolver_peak_bins = peakBins(convSamples, 16);
          f.audio_convolver_ok = true;
          f.audio_convolver_algo = "gr_audio_convolver_iir_v1";
          f.audio_convolver_digest = simpleHash(
            f.audio_convolver_curve.join(",") +
              "|" +
              (f.audio_convolver_moments.mean || 0) +
              "|" +
              (f.audio_convolver_moments.rms || 0)
          ).slice(0, 12);
        } else {
          f.audio_convolver_ok = false;
          f.audio_convolver_skip = convSamples ? "short" : "failed";
        }
        f.audio_deep_hash = simpleHash(
          f.audio_deep_curve.join(",") +
            "|" +
            (m.mean || 0) +
            "|" +
            (m.rms || 0) +
            "|" +
            (f.audio_deep_phase_digest || "") +
            "|" +
            (f.audio_convolver_digest || "")
        ).slice(0, 12);
        // iss/58 A6: spectrum-style bins + waveshaper oversample signature
        try {
          var fftBins = [];
          var step = Math.floor(a.length / 32) || 1;
          var bi;
          for (bi = 0; bi < 32; bi++) {
            var s0 = 0, s1 = 0, j;
            for (j = 0; j < step; j++) {
              var v = a[bi * step + j] || 0;
              s0 += v; s1 += v * v;
            }
            fftBins.push(round6(s0 / step));
            fftBins.push(round6(Math.sqrt(s1 / step)));
          }
          f.audio_fft_curve = fftBins.slice(0, 48);
          f.audio_fft_digest = simpleHash(fftBins.join(",")).slice(0, 12);
          f.audio_waveshaper_oversample = "4x";
          f.audio_src_digest = simpleHash(
            f.audio_fft_digest + "|4x|" + (f.audio_convolver_digest || "")
          ).slice(0, 12);
        } catch (eA6) {
          f.audio_a6_skip = String((eA6 && eA6.message) || eA6);
        }
        f.audio_deep_algo = "gr_d03_audio_deep_v4_multiseed_convolver_a6";
        f.data_ok = a.length > 1000;
        enqueue(ctx, "B46_audio_deep", f, 35, "main");
      }
      try {
        var Offline = window.OfflineAudioContext || window.webkitOfflineAudioContext;
        if (!Offline) {
          f.audio_deep_skip = "no_OfflineAudioContext";
          f.data_ok = true;
          enqueue(ctx, "B46_audio_deep", f, 35, "main");
          return;
        }
        var sampleRate = 44100;
        var length = 22050; // 0.5s
        var seedA = { f1: 1000, f2: 500, lp: 3500, q: 1.2, t1: "triangle", t2: "sawtooth" };
        var seedB = { f1: 1337, f2: 641, lp: 2800, q: 0.9, t1: "sine", t2: "triangle" };
        // Serial audio path (resource class audio exclusive): seedA → seedB → convolver
        renderSeed(Offline, sampleRate, length, seedA)
          .then(function (ra) {
            return renderSeed(Offline, sampleRate, length, seedB).then(function (rb) {
              return renderConvolverIir(Offline, sampleRate, length)
                .then(function (conv) {
                  finishDual(ra.samples, rb.samples, ra.sr || rb.sr, conv);
                })
                .catch(function () {
                  // Convolver optional — dual-seed still valuable
                  finishDual(ra.samples, rb.samples, ra.sr || rb.sr, null);
                });
            });
          })
          .catch(function (e) {
            f.audio_deep_error = String((e && e.message) || e);
            f.data_ok = false;
            enqueue(ctx, "B46_audio_deep", f, 35, "main");
          });
      } catch (e) {
        f.audio_deep_error = String((e && e.message) || e);
        f.data_ok = false;
        enqueue(ctx, "B46_audio_deep", f, 35, "main");
      }
    },
  });
  register("B47_sab_clock", {
    priority: 62,
    schedule: "dynamic",
    batch_id: "B47_sab_clock",
    layer: "mid",
    run: function (ctx) {
      var f = {
        sab_clock_algo: "gr_sab_clock_v1",
        pohw_direction: "A3",
        collected_at: Date.now(),
        cross_origin_isolated:
          typeof crossOriginIsolated !== "undefined" ? !!crossOriginIsolated : false,
        has_shared_array_buffer: typeof SharedArrayBuffer !== "undefined",
        has_atomics: typeof Atomics !== "undefined",
      };
      function finish(ok) {
        f.data_ok = ok !== false;
        enqueue(ctx, "B47_sab_clock", f, 62, "main");
      }
      if (!f.cross_origin_isolated) {
        f.sab_clock_skip = "need_coop_coep_cross_origin_isolated";
        f.sab_clock_ok = false;
        finish(true); // honest skip is ok terminal
        return;
      }
      if (!f.has_shared_array_buffer || !f.has_atomics) {
        f.sab_clock_skip = "no_sab_or_atomics";
        f.sab_clock_ok = false;
        finish(true);
        return;
      }
      try {
        // Worker spins Atomics.add; main samples counter for calibrated clock.
        var sab = new SharedArrayBuffer(8);
        var view = new Int32Array(sab);
        Atomics.store(view, 0, 0);
        Atomics.store(view, 1, 0); // control: 0=run 1=stop
        var workerSrc =
          "onmessage=function(e){" +
          "var v=new Int32Array(e.data);" +
          "while(Atomics.load(v,1)===0){Atomics.add(v,0,1);}" +
          "postMessage(Atomics.load(v,0));" +
          "};";
        var blob = new Blob([workerSrc], { type: "application/javascript" });
        var url = URL.createObjectURL(blob);
        var w = new Worker(url);
        var tStart =
          typeof performance !== "undefined" && performance.now
            ? performance.now()
            : Date.now();
        w.postMessage(sab);
        // Calibrate: sample over ~8ms wall
        var samples = [];
        var nSample = 0;
        function sampleOnce() {
          var t =
            typeof performance !== "undefined" && performance.now
              ? performance.now()
              : Date.now();
          var c = Atomics.load(view, 0);
          samples.push({ t: t, c: c });
          nSample++;
          if (nSample < 12) {
            // ~0.7ms spacing
            setTimeout(sampleOnce, 0);
          } else {
            Atomics.store(view, 1, 1); // stop worker
            setTimeout(function () {
              try {
                w.terminate();
              } catch (eT) {}
              try {
                URL.revokeObjectURL(url);
              } catch (eU) {}
              // ticks per ms from first→last sample with c growth
              var i0 = 0;
              var i1 = samples.length - 1;
              while (i0 < i1 && samples[i0 + 1].c <= samples[i0].c) i0++;
              while (i1 > i0 && samples[i1].c <= samples[i1 - 1].c) i1--;
              var dt = samples[i1].t - samples[i0].t;
              var dc = samples[i1].c - samples[i0].c;
              var tpm = dt > 0 && dc > 0 ? dc / dt : 0;
              // event-loop jitter of sampling intervals (ms)
              var intervals = [];
              var j;
              for (j = 1; j < samples.length; j++) {
                intervals.push(samples[j].t - samples[j - 1].t);
              }
              var meanI = 0;
              for (j = 0; j < intervals.length; j++) meanI += intervals[j];
              meanI = intervals.length ? meanI / intervals.length : 0;
              var varI = 0;
              for (j = 0; j < intervals.length; j++) {
                var d = intervals[j] - meanI;
                varI += d * d;
              }
              var stdI = intervals.length ? Math.sqrt(varI / intervals.length) : 0;
              // estimated resolution (ns): wall ms / ticks * 1e6
              var resNs = tpm > 0 ? Math.round((1 / tpm) * 1e6) : null;
              f.sab_clock_ok = tpm > 1000;
              f.sab_ticks_per_ms = Math.round(tpm);
              f.sab_clock_res_ns = resNs;
              f.sab_eventloop_jitter_ms = Math.round(stdI * 1000) / 1000;
              f.sab_eventloop_mean_ms = Math.round(meanI * 1000) / 1000;
              f.sab_sample_n = samples.length;
              f.sab_calibrate_ms = Math.round((samples[i1].t - tStart) * 100) / 100;
              // compact curve for tz/tm stack (relative tick deltas)
              var curve = [];
              for (j = 1; j < samples.length; j++) {
                curve.push(samples[j].c - samples[j - 1].c);
              }
              f.sab_tick_delta_curve = curve.slice(0, 16);
              f.sab_clock_digest = simpleHash(
                [
                  f.sab_ticks_per_ms,
                  f.sab_clock_res_ns,
                  f.sab_eventloop_jitter_ms,
                  curve.slice(0, 8).join(","),
                ].join("|")
              ).slice(0, 12);
              // Also expose aliases expected by device_segments timing_stack_extra
              f.sab_timer_hash = f.sab_clock_digest;
              f.atomics_timing_digest = f.sab_clock_digest;
              f.perf_now_resolution_ms = resNs != null ? resNs / 1e6 : null;
              finish(true);
            }, 20);
          }
        }
        // let worker spin up
        setTimeout(sampleOnce, 4);
      } catch (e) {
        f.sab_clock_ok = false;
        f.sab_clock_error = String((e && e.message) || e);
        f.sab_clock_skip = "exception";
        finish(true);
      }
    },
  });
  register("B81_webcodecs_bitstream", {
    priority: 48, schedule: "dynamic", batch_id: "B81_webcodecs_bitstream", layer: "mid",
    run: function (ctx) {
      var f = { vc_probe_algo: "gr_webcodecs_bitstream_v1", collected_at: Date.now() };
      function done(ok) { f.vc_ok=!!ok; f.data_ok=true; enqueue(ctx,"B81_webcodecs_bitstream",f,48,"main"); return f; }
      try {
        if (typeof VideoEncoder==="undefined"||typeof VideoFrame==="undefined"){ f.vc_skip="no_webcodecs"; return Promise.resolve(done(false)); }
        var chunks=[], times=[], canvas=typeof OffscreenCanvas!=="undefined"?new OffscreenCanvas(64,64):document.createElement("canvas");
        if(canvas.width!=null){canvas.width=64;canvas.height=64;}
        var g=canvas.getContext("2d"); if(!g){f.vc_skip="no_2d";return Promise.resolve(done(false));}
        var enc=new VideoEncoder({output:function(chunk){try{var buf=new Uint8Array(chunk.byteLength);chunk.copyTo(buf);var acc=0,i;for(i=0;i<Math.min(64,buf.length);i++)acc=(acc*33+buf[i])>>>0;chunks.push(acc);}catch(e){}},error:function(){f.vc_err="encoder_error";}});
        try{enc.configure({codec:"avc1.42E01E",width:64,height:64,bitrate:1e5,framerate:15,hardwareAcceleration:"prefer-hardware",avc:{format:"annexb"}});}
        catch(e){f.vc_skip="configure_failed";try{enc.close();}catch(e0){}return Promise.resolve(done(false));}
        var fi=0;
        return new Promise(function(resolve){function one(){if(fi>=8){enc.flush().then(function(){try{enc.close();}catch(e1){}f.vc_chunk_n=chunks.length;f.vc_encode_timing_curve=times;f.vc_encoder_bitstream_digest=simpleHash(chunks.join(",")).slice(0,12);resolve(done(chunks.length>0));}).catch(function(){f.vc_skip="flush_failed";resolve(done(false));});return;}
          g.fillStyle="rgb("+((fi*37)%255)+","+((fi*91)%255)+",40)";g.fillRect(0,0,64,64);g.fillStyle="#fff";g.fillText("gr"+fi,4+fi,24);
          var t0=performance.now();try{var frame=new VideoFrame(canvas,{timestamp:fi*66666});enc.encode(frame,{keyFrame:fi===0});frame.close();times.push(Math.round((performance.now()-t0)*100)/100);}catch(eE){f.vc_skip="encode_throw";resolve(done(false));return;}fi++;setTimeout(one,0);}one();});
      }catch(e){f.vc_skip="exception";return Promise.resolve(done(false));}
    }
  });
  register("B82_idb_write_ladder", {
    priority: 42, schedule: "dynamic", batch_id: "B82_idb_write_ladder", layer: "mid",
    run: function(ctx){
      var f={idb_probe_algo:"gr_idb_write_ladder_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B82_idb_write_ladder",f,42,"main");return f;}
      if(typeof indexedDB==="undefined"){f.idb_skip="no_idb";return Promise.resolve(done());}
      var sizes=[1024,4096,16384,65536,262144],times=[],dbName="gr_idb_"+Date.now();
      return new Promise(function(resolve){var req=indexedDB.open(dbName,1);req.onupgradeneeded=function(){try{req.result.createObjectStore("b");}catch(e){}};req.onerror=function(){f.idb_skip="open_failed";resolve(done());};
        req.onsuccess=function(){var db=req.result,si=0;function step(){if(si>=sizes.length){try{db.close();indexedDB.deleteDatabase(dbName);}catch(e){}f.idb_write_timing_curve=times;f.idb_write_digest=simpleHash(times.map(function(t){return Math.round(t*10)/10;}).join(",")).slice(0,12);f.idb_ok=true;resolve(done());return;}
          var n=sizes[si++],payload=new Uint8Array(n),i;for(i=0;i<n;i++)payload[i]=(i*17+n)&255;var t0=performance.now();
          try{var tx=db.transaction("b","readwrite");tx.objectStore("b").put(payload,"k"+n);tx.oncomplete=function(){times.push(Math.round((performance.now()-t0)*100)/100);setTimeout(step,0);};tx.onerror=function(){times.push(-1);setTimeout(step,0);};}catch(e){times.push(-1);setTimeout(step,0);}}step();};});
    }
  });
  register("B83_eventloop_signature", {
    priority: 40, schedule: "dynamic", batch_id: "B83_eventloop_signature", layer: "mid",
    run: function(ctx){
      var f={ev_probe_algo:"gr_eventloop_sig_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B83_eventloop_signature",f,40,"main");return f;}
      return new Promise(function(resolve){var samples=[],sch=typeof scheduler!=="undefined"?scheduler:null;
        function finishRaf(){var raf=[],n=0,last=0;function tick(ts){if(last)raf.push(Math.round((ts-last)*100)/100);last=ts;n++;if(n<12)requestAnimationFrame(tick);else{f.raf_spacing_curve=raf;f.scheduler_timing_curve=samples;f.eventloop_digest=simpleHash(samples.join(",")+"|"+raf.join(",")).slice(0,12);resolve(done());}}requestAnimationFrame(tick);}
        if(sch&&typeof sch.postTask==="function"){var chain=Promise.resolve();["user-blocking","user-visible","background"].forEach(function(p){chain=chain.then(function(){return sch.postTask(function(){samples.push(Math.round(performance.now()*100)/100);},{priority:p});});});
          chain.then(function(){f.scheduler_posttask_ok=true;}).catch(function(){f.scheduler_posttask_ok=false;}).then(finishRaf);}else{f.scheduler_posttask_ok=false;finishRaf();}
      });
    }
  });

  C.__midGpuLoaded = true;
  C.__midFamilies = C.__midFamilies || {};
  C.__midFamilies.gpu = true;
  // Full mid loaded if all three families present (compat).
  if (C.__midFamilies.core && C.__midFamilies.gpu && C.__midFamilies.misc) {
    C.__midLoaded = true;
  }
})(typeof window !== "undefined" ? window : globalThis);
