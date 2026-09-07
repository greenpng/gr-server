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
   * Keep Standard-B content-hashed URLs intact; keep `/dist/v/<ver>/g/<gen>/`
   * path segments (1.0.3+: version-keyed paths rotate caches across releases).
   * Relative logical names join asset_base (version-keyed from manifest) +
   * gen-injected filename.
   */
  function withProductVer(src) {
    src = String(src || "");
    if (!src) return src;
    // Standard C: already opaque/hashed basename — keep verbatim (version
    // segments preserved: cache rotation lives in the real path, not queries).
    if (isHashedOrOpaqueLeaf(src)) {
      return src;
    }
    // Prefer absolute asset_base from manifest when src is relative.
    try {
      var base = assetBase();
      if (base && src.charAt(0) !== "/" && src.indexOf("://") < 0) {
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
