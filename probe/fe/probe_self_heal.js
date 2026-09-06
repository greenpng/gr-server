/**
 * GR / green-v6 Probe Self-Heal — Gap Table + Supervisor + loops T/C/B + adaptive.
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
