/**
 * Domain module: engine-targeted residual deepen (B10x_*).
 *
 * Load AFTER main registry (needs midFields, midEnqueue, webglResidualMultiPath).
 * Coexists with B10_hw_curves — residual_paths union on server.
 *
 * @see fe/collectors/DOMAINS.md
 * @see v5-docs/architecture/pack-budget-tiers.md
 */
(function (global) {
  "use strict";

  function webglStackMarkers() {
    var m = {
      webgl_support: false,
      webgl2_support: false,
      webgl_unmasked_vendor: "",
      webgl_unmasked_renderer: "",
      webgl_context_type: null,
      webgl_extensions_count: null,
      webgl_unmask_blocked: false,
      gl_stack_class: "unknown",
    };
    try {
      // Prefer shared acquire (releases all first) to avoid context pile-up.
      var acq = null;
      var h = null;
      try {
        h = global.GRCollectors && global.GRCollectors.__h;
        if (h && typeof h.acquireProbeGl === "function") {
          acq = h.acquireProbeGl(4, { glOpts: { antialias: false } });
        }
      } catch (eA) {}
      var gl = acq && acq.gl;
      var c = acq && acq.canvas;
      if (!gl) {
        try {
          if (h && typeof h.releaseWebglProbeContexts === "function") h.releaseWebglProbeContexts();
        } catch (eR0) {}
        c = document.createElement("canvas");
        c.width = 4;
        c.height = 4;
        var gl2 = null;
        try {
          gl2 = c.getContext("webgl2", { antialias: false });
        } catch (e2) {}
        gl =
          gl2 ||
          c.getContext("webgl", { antialias: false }) ||
          c.getContext("experimental-webgl", { antialias: false });
      }
      if (!gl) return m;
      m.webgl_support = true;
      try {
        m.webgl2_support =
          (typeof WebGL2RenderingContext !== "undefined" && gl instanceof WebGL2RenderingContext) ||
          !!(gl && typeof gl.createVertexArray === "function");
      } catch (eW2) {
        m.webgl2_support = false;
      }
      m.webgl_context_type = m.webgl2_support ? "webgl2" : "webgl";
      try {
        var exts = gl.getSupportedExtensions();
        m.webgl_extensions_count = exts ? exts.length : 0;
      } catch (eE) {}
      try {
        var dbg = gl.getExtension("WEBGL_debug_renderer_info");
        if (dbg) {
          m.webgl_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) || "";
          m.webgl_unmasked_renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "";
        }
      } catch (eU) {}
      var vl = String(m.webgl_unmasked_vendor || "").toLowerCase();
      var rl = String(m.webgl_unmasked_renderer || "").toLowerCase();
      if (vl.indexOf("google") >= 0 || rl.indexOf("angle") >= 0) m.gl_stack_class = "angle";
      else if (vl.indexOf("apple") >= 0) {
        m.gl_stack_class = rl ? "webkit_or_apple" : "webkit_unmask_blocked";
        m.webgl_unmask_blocked = !rl;
      } else if (
        rl.indexOf("llvmpipe") >= 0 ||
        rl.indexOf("swiftshader") >= 0 ||
        rl.indexOf("softpipe") >= 0
      ) {
        m.gl_stack_class = "software_gl";
      } else if (vl.indexOf("nvidia") >= 0 || vl.indexOf("amd") >= 0 || vl.indexOf("intel") >= 0) {
        m.gl_stack_class = "native_desktop_gl";
      }
    } catch (e0) {}
    // Always release after markers — never leave stack-marker context live.
    try {
      releaseGpuAfterProbe();
    } catch (eRel) {}
    return m;
  }

  function releaseGpuAfterProbe() {
    try {
      var h = global.GRCollectors && global.GRCollectors.__h;
      if (h && typeof h.releaseWebglProbeContexts === "function") {
        h.releaseWebglProbeContexts();
        return;
      }
    } catch (eH) {}
    try {
      var gl = global.__GR_LAST_GL__;
      if (gl) {
        try {
          var lose = gl.getExtension && gl.getExtension("WEBGL_lose_context");
          if (lose) /*lose_suppressed*/void 0;
        } catch (eL) {}
        global.__GR_LAST_GL__ = null;
      }
    } catch (e0) {}
    try {
      var list = global.__GR_PROBE_CANVASES__ || [];
      for (var i = 0; i < list.length; i++) {
        try {
          var c = list[i];
          if (c && c.width != null) {
            c.width = 1;
            c.height = 1;
          }
        } catch (eC) {}
      }
      global.__GR_PROBE_CANVASES__ = [];
      global.__GR_PROBE_GL__ = [];
    } catch (e1) {}
    try {
      if (global.gc) global.gc();
    } catch (e2) {}
  }

  function withTimeout(p, ms, label) {
    return new Promise(function (resolve) {
      var done = false;
      var to = setTimeout(function () {
        if (done) return;
        done = true;
        resolve({ __timeout: true, label: label || "timeout" });
      }, ms || 18000);
      Promise.resolve(p).then(
        function (v) {
          if (done) return;
          done = true;
          clearTimeout(to);
          resolve(v);
        },
        function (e) {
          if (done) return;
          done = true;
          clearTimeout(to);
          resolve({ __error: String(e && e.message ? e.message : e) });
        }
      );
    });
  }

  function runB10xResidualPack(C, ctx, packId, profile, prio) {
    // Progressive load puts helpers on C.__h (static lite), not always C.midFields.
    var h = (C && C.__h) || {};
    var midFields = C.midFields || h.midFields;
    var midEnqueue = C.midEnqueue || h.midEnqueue;
    var webglResidualMultiPath = C.webglResidualMultiPath || h.webglResidualMultiPath;
    var detectEngineFamily =
      C.detectEngineFamily || h.detectEngineFamily || function () { return "unknown"; };
    var probeProfileForEngine =
      C.probeProfileForEngine || h.probeProfileForEngine || function () { return "probe_unknown_v1"; };
    if (typeof midFields !== "function" || typeof midEnqueue !== "function") {
      try {
        if (global.GROps && GROps.report) {
          GROps.report(
            "b10x_helpers_missing",
            "kick",
            { pack_id: String(packId || ""), has_h: !!C.__h },
            "error"
          );
        }
      } catch (eH) {}
      // Honest skip pack even when helpers missing (direct queue if needed).
      try {
        var Q0 = (ctx && ctx.queue) || global.GRUploadQueue;
        var sid0 = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
        if (Q0 && Q0.enqueue && sid0) {
          Q0.enqueue({
            session_id: sid0,
            batch_id: packId,
            source: "main",
            inject_path: (ctx && ctx.inject_path) || "",
            priority: prio || 86,
            force: true,
            force_after_halt: true,
            payload: {
              fields: {
                b10x_pack: packId,
                b10x_ok: false,
                b10x_err: "helpers_missing",
                b10x_phase: "done",
                residual_paths_n: 0,
              },
              sandbox_kind: "main",
            },
          });
        }
      } catch (eSkip) {}
      return Promise.resolve({ batch_id: packId, error: "helpers_missing" });
    }
    var f = midFields(packId || "b10x");
    f.engine_family = detectEngineFamily();
    f.probe_profile = probeProfileForEngine(f.engine_family);
    f.b10x_pack = packId;
    f.b10x_profile = profile;
    f.b10x_phase = "start";
    // Product principle: do NOT skip packs based on claimed OS/BR/engine.
    // All B10x variants still run under HW lock + single-GL-context discipline.
    // Only true missing WebGL is an honest capability skip (no multipath thrash).
    var markers = webglStackMarkers();
    Object.keys(markers).forEach(function (k) {
      f[k] = markers[k];
    });
    if (markers && markers.webgl_support === false) {
      f.b10x_ok = false;
      f.b10x_err = "no_webgl";
      f.b10x_capability = "unsupported";
      f.b10x_phase = "done";
      f.residual_paths_n = 0;
      return midEnqueue(ctx, packId, f, prio || 86);
    }
    // Immediate started marker so hang still leaves a batch — but never re-stamp
    // after this pack already sealed (was doubling wire with force:true storms).
    try {
      var Qs = (ctx && ctx.queue) || global.GRUploadQueue;
      var sids = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
      var alreadyStart =
        Qs &&
        Qs.alreadySent &&
        sids &&
        Qs.alreadySent({ session_id: sids, batch_id: packId, source: "main" });
      if (Qs && Qs.enqueue && sids && !alreadyStart) {
        Qs.enqueue({
          session_id: sids,
          batch_id: packId,
          source: "main",
          inject_path: (ctx && ctx.inject_path) || "",
          priority: prio || 86,
          force: true,
          force_after_halt: true,
          payload: {
            fields: Object.assign({}, f, {
              b10x_ok: false,
              b10x_err: "started",
              b10x_phase: "start",
            }),
            sandbox_kind: "main",
          },
        });
      }
    } catch (eStart) {}
    if (typeof webglResidualMultiPath !== "function") {
      f.b10x_ok = false;
      f.b10x_err = "no_multipath";
      f.b10x_phase = "done";
      f.residual_paths_n = 0;
      return midEnqueue(ctx, packId, f, prio || 86);
    }
    var runMp = function () {
      return withTimeout(webglResidualMultiPath({ profile: profile }), 20000, "b10x_mp")
        .then(function (mp) {
          if (!mp || mp.__timeout || mp.__error) {
            f.b10x_ok = false;
            f.b10x_err = (mp && mp.__timeout) ? "multipath_timeout" : ((mp && mp.__error) || "multipath_empty");
            f.b10x_timeout = !!(mp && mp.__timeout);
            f.b10x_phase = "done";
            f.residual_paths_n = f.residual_paths_n != null ? f.residual_paths_n : 0;
            return midEnqueue(ctx, packId, f, prio || 86);
          }
          f.b10x_ok = !!(mp.curve && mp.curve.length >= 8);
          f.residual_paths = mp.residual_paths || [];
          f.residual_select = mp.residual_select || null;
          f.residual_probe_engine = mp.residual_probe_engine || f.engine_family;
          f.residual_probe_profile = mp.residual_probe_profile || f.probe_profile;
          f.multipath_profile = mp.multipath_profile || profile;
          f.webgl_probe_path = "b10x_" + profile + "_async";
          // Lane-S packs must not last-write commercial residual_mean / hw_curve_webgl
          // (mint previously followed fbs.main last pack → incomplete 0900/e432 cluster).
          var laneSOnly =
            profile === "silicon_deep" || profile === "silicon_ulp";
          if (mp.curve && mp.curve.length) {
            f.hw_noise_curves = f.hw_noise_curves || {};
            if (laneSOnly) {
              f.hw_curve_webgl_lane_s = mp.curve;
              f.hw_noise_curves.webgl_lane_s = mp.curve;
            } else {
              f.hw_curve_webgl = mp.curve;
              f.hw_noise_curves.webgl = mp.curve;
            }
          }
          if (mp.residual_mean != null) {
            if (laneSOnly) {
              f.residual_mean_lane_s = mp.residual_mean;
            } else {
              f.residual_mean = mp.residual_mean;
              f.residual_available = true;
            }
          }
          if (mp.residual_std != null) {
            if (laneSOnly) f.residual_std_lane_s = mp.residual_std;
            else f.residual_std = mp.residual_std;
          }
          // Commercial residual_algo must follow Lane-C priority (noderiv > float > rint).
          // Lower-priority packs (e.g. rint after noderiv) must not last-write the label
          // while digests are sealed from the primary path (iss/75 178 ops mismatch).
          if (mp.residual_algo) {
            if (laneSOnly) {
              f.residual_algo_lane_s = mp.residual_algo;
            } else {
              var rankAlgo = function (a) {
                a = String(a || "").toLowerCase();
                if (a.indexOf("noderiv") >= 0) return 0;
                if (a.indexOf("rint") >= 0) return 2;
                if (a.indexOf("float") >= 0) return 1;
                return 3;
              };
              if (
                f.residual_algo == null ||
                rankAlgo(mp.residual_algo) < rankAlgo(f.residual_algo)
              ) {
                f.residual_algo = mp.residual_algo;
              }
            }
          }
          f.residual_paths_n = (f.residual_paths || []).length;
          f.residual_ok =
            (f.residual_std != null && Number(f.residual_std) > 0) ||
            (f.residual_mean != null && f.residual_available === true) ||
            !!(f.hw_curve_webgl && f.hw_curve_webgl.length >= 8) ||
            f.residual_paths_n > 0;
          if (!f.b10x_ok && !f.b10x_err) {
            f.b10x_err = f.residual_paths_n > 0 ? "weak_curve" : "no_curve";
          }
          f.b10x_phase = "done";
          f.b10x_meta = {
            pack_id: packId,
            profile: profile,
            n_paths: f.residual_paths_n,
            chosen: f.residual_select && f.residual_select.chosen_path_id,
            mean: f.residual_mean,
            std: f.residual_std,
            gl_stack_class: f.gl_stack_class,
            unmask_blocked: !!f.webgl_unmask_blocked,
          };
          return midEnqueue(ctx, packId, f, prio || 86);
        })
        .catch(function (e) {
          f.b10x_ok = false;
          f.b10x_err = String(e && e.message ? e.message : e);
          f.b10x_phase = "done";
          f.residual_paths_n = f.residual_paths_n != null ? f.residual_paths_n : 0;
          return midEnqueue(ctx, packId, f, prio || 86);
        })
        .then(function (r) {
          releaseGpuAfterProbe();
          return r;
        });
    };
    // Serialize with process HW lock when available (stagger vs B10/B7/other B10x).
    try {
      var PL = global.GRPackLoader;
      if (PL && typeof PL.withHardwareLock === "function") {
        return PL.withHardwareLock("b10x:" + packId, runMp);
      }
    } catch (eLock) {}
    return runMp();
  }

  function registerAll(C) {
    var register = C.register.bind(C);
    var packs = [
      ["B10x_webkit_gl_noise", "webkit_deep", 88],
      ["B10x_webkit_wave2_warm4", "webkit_deep", 86],
      ["B10x_softgl_hedge", "softgl", 84],
      ["B10x_legacy_webgl1", "legacy_webgl1", 83],
      ["B10x_unknown_kernel", "unknown", 87],
      ["B10x_angle_crosscheck", "angle_cross", 80],
      // Silicon: Lane-C first (noderiv>rint), then Lane-S deepen (ulp/deep last).
      ["B10x_silicon_noderiv", "silicon_noderiv", 94],
      ["B10x_silicon_rint", "silicon_rint", 93],
      ["B10x_silicon_ulp", "silicon_ulp", 90],
      // iss/54 P1–P4: fma/denorm/tex — after Lane-C sealed (lower prio)
      ["B10x_silicon_deep", "silicon_deep", 86],
    ];
    packs.forEach(function (row) {
      var id = row[0];
      var profile = row[1];
      var prio = row[2];
      // Domain pack upgrades hard-inline (fuller honesty + force_after_halt).
      // Keep hard-inline only when domain helpers are broken (runB10xResidualPack guards that).
      register(id, {
        priority: prio,
        schedule: "dynamic",
        batch_id: id,
        layer: "hard",
        pack_lane: "deepen",
        run: function (ctx) {
          return runB10xResidualPack(C, ctx, id, profile, prio);
        },
      });
    });
    try {
      C.webglStackMarkers = webglStackMarkers;
      C.runB10xResidualPack = function (ctx, packId, profile, prio) {
        return runB10xResidualPack(C, ctx, packId, profile, prio);
      };
    } catch (eX) {}
  }

  function tryBoot() {
    try {
      if (global.GRCollectors && global.GRCollectors.register) {
        registerAll(global.GRCollectors);
        return true;
      }
    } catch (e) {}
    return false;
  }

  if (!tryBoot()) {
    var n = 0;
    var t = setInterval(function () {
      n++;
      if (tryBoot() || n > 200) clearInterval(t);
    }, 25);
  }
})(typeof window !== "undefined" ? window : globalThis);
