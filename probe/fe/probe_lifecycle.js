/**
 * GR probe lifecycle — thin FE self-consistency (no heavy brain in browser).
 *
 * States: idle_cool | need_probe | probing | retry_backoff | failed_quiet | complete_cool
 * Authority: product_version + server cool/final; FE only gap-fills and backs off.
 *
 * @see reports/FE_PROBE_LIFECYCLE_DESIGN_V5852.md
 */
(function (global) {
  "use strict";

  var STATE = {
    IDLE_COOL: "idle_cool",
    NEED_PROBE: "need_probe",
    PROBING: "probing",
    RETRY_BACKOFF: "retry_backoff",
    FAILED_QUIET: "failed_quiet",
    COMPLETE_COOL: "complete_cool",
  };

  /**
   * Hard anchors: more retries. Soft: fewer then drop pack (not whole cycle).
   * multi_tick_max is a SAFETY CAP only — authority to stop is server
   * stop_probe+coverage / brain_schedule_final / 410 (see FE_BE_STATE_MACHINE_AUTHORITY_V5855).
   */
  var POLICY = {
    /** Hard anchors (B10/B0/B2…): enough retries for seal races + short network blips. */
    hard_max_attempts: 10,
    soft_max_attempts: 3,
    /** Silicon deepen (B10x_*): short-visit front-loaded retries (network blips). */
    deepen_max_attempts: 8,
    /** Continuous RPA (B11): tight cap — never soft_exhausted storms. */
    rpa_max_attempts: 2,
    /**
     * Heavy (B10x/B7) concurrency. Keep 1 under real visitor networks —
     * prod v150 zhanso: concurrent B10x+B47 → Failed to fetch / hard_exhausted mis-tag.
     */
    heavy_max_inflight: 1,
    /** ms delays by attempt index (1-based → index 0) */
    /** Front-load hard (B10) retries for short dwell + visitor network blips. */
    hard_backoff_ms: [200, 500, 1100, 2200, 4500, 9000, 16000, 24000],
    soft_backoff_ms: [2000, 6000, 15000],
    /** Front-load silicon deepen retries for short dwell + visitor network. */
    deepen_backoff_ms: [200, 500, 1200, 2800, 6000, 12000, 20000],
    /** Lane-C must-land (noderiv/rint/ulp) — tighter than generic deepen. */
    commercial_land_backoff_ms: [120, 360, 680, 1100, 1600, 2000],
    rpa_backoff_ms: [3000, 12000],
    /** global fails in rolling window before soft quiet */
    fail_budget_n: 18,
    fail_budget_window_ms: 90000,
    /** After RPA exhaust, quiet only that pack class (ms). */
    rpa_quiet_ms: 90000,
    /**
     * After deepen exhaust quiet — shorter so B10-landed re-kick lands B10x same session.
     */
    deepen_quiet_ms: 22000,
    /**
     * hard SLA re-kick for missing B10 — front-load for short visits (p50 dwell ~2–4s).
     * Extended tail so multi-tab / background multipath can still re-arm without refresh.
     * Long-window collect is also owned by GRProbeSelfHeal Loop C.
     */
    hard_sla_delays_ms: [800, 2200, 5500, 12000, 20000, 45000, 90000],
    /**
     * iss/70: multi_tick_max is abnormal safety only.
     * Normal path stops earlier via plan_epoch / empty_kick / server terminal.
     */
    multi_tick_max: 16,
    /** Wait ticks with 0 new kicks before giving up. */
    empty_kick_patience: 5,
    /** After B10 lands, even fewer empty plan ticks before stop. */
    empty_kick_patience_post_b10: 3,
    upload_max_retries: 5,
    client_alive_retry_ms: 30000,
    /**
     * Upload concurrency base (iss/70 P2: final cap = min of budgets, not max overlay).
     * Adaptive AIMD may raise up to upload_concurrency_max.
     */
    upload_concurrency: 3,
    upload_concurrency_max: 6,
    upload_mid_ramp: 4,
    upload_ramp_after: 4,
  };

  /**
   * Apply server open.policy.fe_retry (admin panel publish). Unknown keys ignored.
   */
  function applyServerPolicy(pol) {
    if (!pol || typeof pol !== "object") return POLICY;
    var keys = [
      "hard_max_attempts",
      "soft_max_attempts",
      "deepen_max_attempts",
      "rpa_max_attempts",
      "heavy_max_inflight",
      "fail_budget_n",
      "fail_budget_window_ms",
      "rpa_quiet_ms",
      "deepen_quiet_ms",
      "multi_tick_max",
      "empty_kick_patience",
      "empty_kick_patience_post_b10",
      "upload_max_retries",
      "client_alive_retry_ms",
      "upload_concurrency",
      "upload_concurrency_max",
      "upload_mid_ramp",
      "upload_ramp_after",
    ];
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i];
      if (typeof pol[k] === "number" && isFinite(pol[k]) && pol[k] > 0) {
        POLICY[k] = Math.floor(pol[k]);
      }
    }
    if (Array.isArray(pol.hard_sla_delays_ms) && pol.hard_sla_delays_ms.length) {
      POLICY.hard_sla_delays_ms = pol.hard_sla_delays_ms.map(function (x) {
        return Math.floor(Number(x) || 5000);
      });
    }
    return POLICY;
  }

  var failTimes = [];
  var state = STATE.NEED_PROBE;
  var quietUntil = 0;

  function now() {
    return Date.now();
  }

  function pruneFails() {
    var t = now();
    var w = POLICY.fail_budget_window_ms;
    failTimes = failTimes.filter(function (x) {
      return t - x < w;
    });
  }

  /**
   * Commercial hard anchors only — B10x silicon deepen is NOT hard.
   * (B10x used to match indexOf("B10") and burn hard_max / hard_exhausted.)
   */
  function isHardBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return false;
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    var hard = [
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
    for (var i = 0; i < hard.length; i++) if (hard[i] === id) return true;
    return false;
  }

  /** Silicon deepen / multipath (scheduled by brain maximize; soft budget). */
  function isDeepenBatch(batchId) {
    var id = String(batchId || "");
    return id.indexOf("B10x_") === 0 || id === "B15_cross_curves";
  }

  /** Large body / GPU / SAB — serialize inflight (avoid tab freeze / WebGL thrash). */
  function isHeavyBatch(batchId) {
    var id = String(batchId || "");
    if (isDeepenBatch(id)) return true;
    return (
      id === "B7_sandbox" ||
      id === "B10_hw_curves" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B47_sab_clock" ||
      id === "B22_gpu_timer" ||
      id === "mid.curves"
    );
  }

  function isRpaBatch(batchId) {
    var id = String(batchId || "");
    return id === "B11_interaction" || id.indexOf("B11_") === 0 || id === "rpa.interaction";
  }

  var rpaQuietUntil = 0;
  var deepenQuietUntil = 0;

  function maxAttemptsFor(batchId) {
    if (isRpaBatch(batchId)) return POLICY.rpa_max_attempts || 2;
    if (isHardBatch(batchId)) return POLICY.hard_max_attempts;
    if (isDeepenBatch(batchId)) return POLICY.deepen_max_attempts || 4;
    return POLICY.soft_max_attempts;
  }

  function heavyMaxInflight() {
    return POLICY.heavy_max_inflight || 1;
  }

  function isCommercialLandFast(batchId) {
    var id = String(batchId || "");
    return (
      id === "B10_hw_curves" ||
      id === "mid.curves" ||
      id === "B10x_silicon_noderiv" ||
      id === "B10x_silicon_rint" ||
      id === "B10x_silicon_ulp"
    );
  }

  function backoffMsFor(batchId, attempt) {
    var a = Math.max(1, attempt | 0);
    var arr;
    if (isCommercialLandFast(batchId)) {
      arr = POLICY.commercial_land_backoff_ms || POLICY.hard_backoff_ms;
    } else if (isRpaBatch(batchId)) {
      arr = POLICY.rpa_backoff_ms || POLICY.soft_backoff_ms;
    } else if (isHardBatch(batchId)) {
      arr = POLICY.hard_backoff_ms;
    } else if (isDeepenBatch(batchId)) {
      arr = POLICY.deepen_backoff_ms || POLICY.soft_backoff_ms;
    } else {
      arr = POLICY.soft_backoff_ms;
    }
    var idx = Math.min(arr.length - 1, a - 1);
    var base = arr[idx] || 2000;
    // small jitter so multi-tab does not sync-storm (skip for first commercial land)
    var j =
      isCommercialLandFast(batchId) && a <= 2
        ? 0
        : Math.floor(base * 0.15 * Math.random());
    return base + j;
  }

  function recordFail(batchId, reason) {
    pruneFails();
    var why = String(reason || "");
    // Timeout aborts / pagehide / soft quiet: do NOT burn fail budget (was flooding
    // probe_fail_budget and blocking mid packs while hard still retrying).
    var softReason =
      /upload_timeout_abort|timeout|pagehide|halt|abort|network/i.test(why) &&
      !/attempts_exhausted/i.test(why);
    if (!softReason) {
      failTimes.push(now());
    }
    // RPA exhaust: quiet only B11 class — do not poison whole soft budget.
    if (why === "attempts_exhausted" && isRpaBatch(batchId)) {
      rpaQuietUntil = now() + (POLICY.rpa_quiet_ms || 90000);
      try {
        if (global.GROps && GROps.report) {
          GROps.report(
            "rpa_quiet",
            "upload",
            {
              batch_id: String(batchId || ""),
              quiet_ms: POLICY.rpa_quiet_ms || 90000,
              reason: why,
            },
            "warn"
          );
        }
      } catch (eR) {}
    }
    // Deepen exhaust / network storm: quiet B10x class so brain re-plan does not spin.
    if (
      (why === "attempts_exhausted" || /network|upload_network/i.test(why)) &&
      isDeepenBatch(batchId)
    ) {
      deepenQuietUntil = now() + (POLICY.deepen_quiet_ms || 120000);
    }
    var over = failTimes.length >= POLICY.fail_budget_n;
    if (over) {
      state = STATE.FAILED_QUIET;
      quietUntil = now() + POLICY.fail_budget_window_ms;
      try {
        // Dedupe ops: at most one probe_fail_budget per window.
        var lastPb = global.__GR_LAST_FAIL_BUDGET_MS__ || 0;
        var tNow = now();
        if (tNow - lastPb > POLICY.fail_budget_window_ms / 2 && global.GROps && GROps.report) {
          global.__GR_LAST_FAIL_BUDGET_MS__ = tNow;
          GROps.report(
            "probe_fail_budget",
            "upload",
            {
              n: failTimes.length,
              window_ms: POLICY.fail_budget_window_ms,
              last_batch: String(batchId || ""),
              reason: why,
            },
            "warn"
          );
        }
      } catch (e) {}
    } else if (!softReason) {
      state = STATE.RETRY_BACKOFF;
    }
    return { overBudget: over, failCount: failTimes.length, rpaQuiet: now() < rpaQuietUntil, soft: softReason };
  }

  function softQuietActive() {
    if (state === STATE.FAILED_QUIET && now() < quietUntil) return true;
    if (state === STATE.FAILED_QUIET && now() >= quietUntil) {
      state = STATE.NEED_PROBE;
      quietUntil = 0;
      failTimes = [];
    }
    return false;
  }

  function rpaQuietActive() {
    if (rpaQuietUntil && now() < rpaQuietUntil) return true;
    if (rpaQuietUntil && now() >= rpaQuietUntil) rpaQuietUntil = 0;
    return false;
  }

  function deepenQuietActive() {
    if (deepenQuietUntil && now() < deepenQuietUntil) return true;
    if (deepenQuietUntil && now() >= deepenQuietUntil) deepenQuietUntil = 0;
    return false;
  }

  /** iss/54–57 secondary silicon/infra — never quiet-block (ok or honest skip must land). */
  function isSecondaryInfraBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    );
  }

  /**
   * Soft packs should not enqueue while fail-budget quiet; hard still may.
   * RPA / deepen have class quiet. Heavy deepen re-queues only after quiet.
   */
  function isTimingSensitiveBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B34_cpu_cache_ladder" ||
      id === "B22_gpu_timer" ||
      id === "B37_thermal_drift_lite" ||
      id === "B42_thermal_drift_full" ||
      /timing|thermal|sab_clock|cache_ladder|eu_timing/i.test(id)
    );
  }

  function allowEnqueue(batchId) {
    if (isSecondaryInfraBatch(batchId)) return true;
    if (isRpaBatch(batchId) && rpaQuietActive()) return false;
    // iss/58 A9: defer timing-sensitive packs under serious/critical pressure
    if (isTimingSensitiveBatch(batchId) && !timingProbesAllowed()) return false;
    // silicon_deep is deepen but must not be blocked by ulp/noderiv quiet
    if (isDeepenBatch(batchId) && deepenQuietActive() && String(batchId || "") !== "B10x_silicon_deep") {
      return false;
    }
    if (softQuietActive() && !isHardBatch(batchId) && !isDeepenBatch(batchId)) return false;
    return true;
  }

  function markProbing() {
    if (state !== STATE.COMPLETE_COOL && state !== STATE.IDLE_COOL) {
      state = STATE.PROBING;
    }
  }

  /**
   * iss/58 A9: Compute Pressure Observer — no user prompt.
   * Gates timing-heavy packs when pressure is serious/critical.
   */
  function installComputePressure() {
    try {
      if (global.__GR_COMPUTE_PRESSURE_INSTALLED__) return;
      global.__GR_COMPUTE_PRESSURE_INSTALLED__ = 1;
      global.__GR_COMPUTE_PRESSURE_STATE__ = "unknown";
      var PO = global.PressureObserver || global.ComputePressureObserver;
      if (!PO) return;
      var obs = new PO(function (records) {
        try {
          var last = records && records.length ? records[records.length - 1] : null;
          var st = (last && (last.state || last.pressure)) || "unknown";
          global.__GR_COMPUTE_PRESSURE_STATE__ = String(st);
        } catch (eR) {}
      });
      // Observe CPU if supported
      try {
        if (obs.observe.length >= 1) {
          var p = obs.observe("cpu");
          if (p && p.catch) p.catch(function () {});
        } else {
          obs.observe({ source: "cpu" });
        }
      } catch (eO) {
        try {
          obs.observe();
        } catch (eO2) {}
      }
    } catch (e) {}
  }

  function timingProbesAllowed() {
    try {
      var st = String(global.__GR_COMPUTE_PRESSURE_STATE__ || "unknown").toLowerCase();
      if (st === "serious" || st === "critical") return false;
    } catch (e) {}
    return true;
  }

  try {
    installComputePressure();
  } catch (eInst) {}

  function markCool(source) {
    state = STATE.COMPLETE_COOL;
    try {
      global.__GR_PHASE__ = "cool";
      global.__GR_BUSINESS_STATE__ = "identity_complete_cool";
      global.__GR_LIFECYCLE_SOURCE__ = source || "cool";
    } catch (e) {}
  }

  function markNeedProbe(reason) {
    state = STATE.NEED_PROBE;
    quietUntil = 0;
    try {
      global.__GR_LIFECYCLE_REASON__ = reason || "need_probe";
    } catch (e) {}
  }

  function deriveFromOpen(opened) {
    opened = opened || {};
    var phase = String(opened.phase || "");
    var skip = !!(opened.skip_identity_probe || opened.skip_session_probe);
    var cps = opened.cycle_probe_status || {};
    var scheduleFinal =
      cps.brain_schedule_final === true ||
      cps.final_analysis_ok === true ||
      cps.cycle_status === "complete" ||
      opened.cycle_status === "complete" ||
      opened.brain_schedule_final === true;
    // Cool only when skip/phase cool AND schedule final (or explicit identity_complete_cool).
    // Thin phase=cool without schedule final → keep probing (gap-fill).
    if (
      (phase === "cool" || skip || opened.business_state === "identity_complete_cool") &&
      (scheduleFinal || opened.business_state === "identity_complete_cool") &&
      !opened.need_hard_anchor &&
      !opened.force_identity_probe
    ) {
      markCool("open");
      return STATE.IDLE_COOL;
    }
    markProbing();
    return STATE.PROBING;
  }

  function snapshot() {
    pruneFails();
    return {
      state: state,
      quiet_until_ms: quietUntil,
      fail_count_window: failTimes.length,
      policy: {
        hard_max_attempts: POLICY.hard_max_attempts,
        soft_max_attempts: POLICY.soft_max_attempts,
        fail_budget_n: POLICY.fail_budget_n,
        multi_tick_max: POLICY.multi_tick_max,
      },
    };
  }

  /**
   * Multi-path re-probe arming (maximize schedule).
   * Triggers: pageshow/BFCache, visibility, focus, storage (other tab),
   * online, cold→hot open force, explicit server route packs.
   * Does NOT stop because commercial device_id already minted.
   */
  var reprobeHooksInstalled = false;
  function installReprobeTriggers(kickFn) {
    if (reprobeHooksInstalled) return;
    reprobeHooksInstalled = true;
    var kick =
      typeof kickFn === "function"
        ? kickFn
        : function (reason) {
            try {
              if (typeof global.grResumeProbeCycle === "function") {
                global.grResumeProbeCycle(reason || "reprobe");
              } else if (typeof global.grKickRoutePlan === "function") {
                global.grKickRoutePlan(reason || "reprobe");
              } else if (global.GRBoot && typeof global.GRBoot.resumeProbe === "function") {
                global.GRBoot.resumeProbe(reason || "reprobe");
              } else {
                global.__GR_NEED_REPROBE__ = true;
                global.__GR_REPROBE_REASON__ = reason || "reprobe";
                markNeedProbe(reason || "reprobe");
              }
            } catch (e) {
              markNeedProbe(reason || "reprobe");
            }
          };
    var lastArmMs = 0;
    var pendingArmReason = null;
    function isBusy() {
      try {
        if (global.__GR_MULTI_TICK_ACTIVE__) return true;
        if (global.GRUploadQueue && GRUploadQueue.stats) {
          var st = GRUploadQueue.stats() || {};
          if ((st.pending || 0) > 0 || (st.inflight || 0) > 0) return true;
        }
      } catch (eB) {}
      return false;
    }
    function arm(reason) {
      try {
        // Never cool solely because a previous session had mint — server schedule owns cool.
        if (state === STATE.COMPLETE_COOL && !global.__GR_FORCE_REPROBE__) {
          return;
        }
        // iss/72 complete form: prefer GRSessionScheduler as single arm/kick arbiter.
        if (global.GRSessionScheduler && typeof GRSessionScheduler.submit === "function") {
          try {
            if (!GRSessionScheduler._lifecycleBound) {
              GRSessionScheduler.bind({
                kick: function (r) {
                  try {
                    markNeedProbe(r);
                    markProbing();
                  } catch (eM) {}
                  kick(r);
                },
                debounce_ms: 5000,
              });
              GRSessionScheduler._lifecycleBound = true;
            }
          } catch (eBind) {}
          var force =
            reason === "server_incomplete" ||
            reason === "route_plan" ||
            !!global.__GR_FORCE_REPROBE__;
          var ep = null;
          try {
            if (global.__GR_PLAN_EPOCH__ != null) ep = Number(global.__GR_PLAN_EPOCH__);
          } catch (eEp) {}
          var sub = GRSessionScheduler.submit(reason || "reprobe", {
            force: force,
            plan_epoch: ep,
          });
          if (sub && sub.ok === false && (sub.reason === "busy" || sub.reason === "debounce")) {
            pendingArmReason = reason || sub.reason;
          }
          return;
        }
        // Fallback lite path when session_scheduler not in pack.
        var now = Date.now();
        if (isBusy()) {
          pendingArmReason = reason || "busy_pending";
          return;
        }
        if (now - lastArmMs < 5000 && reason !== "server_incomplete") {
          pendingArmReason = reason || "debounce";
          return;
        }
        lastArmMs = now;
        pendingArmReason = null;
        markNeedProbe(reason);
        markProbing();
        kick(reason);
      } catch (e) {}
    }
    try {
      global.addEventListener("pageshow", function (ev) {
        arm(ev && ev.persisted ? "bfcache_pageshow" : "pageshow");
      });
      global.addEventListener("visibilitychange", function () {
        if (document.visibilityState === "visible") arm("visibility_visible");
      });
      global.addEventListener("focus", function () {
        arm("window_focus");
      });
      global.addEventListener("online", function () {
        arm("network_online");
      });
      global.addEventListener("storage", function (ev) {
        if (ev && String(ev.key || "").indexOf("gr") >= 0) arm("storage_cross_tab");
      });
      // Periodic soft re-check while incomplete (server may re-arm packs on cold→hot).
      setInterval(function () {
        try {
          if (global.__GR_HALT_UPLOADS__ || global.__GR_BRAIN_SCHEDULE_FINAL__) return;
          if (state === STATE.COMPLETE_COOL) return;
          if (document.visibilityState !== "visible") return;
          // Flush pending arm reason if idle (scheduler tick also drains).
          if (pendingArmReason && !isBusy()) {
            var r = pendingArmReason;
            pendingArmReason = null;
            arm(r);
            return;
          }
          if (global.GRSessionScheduler && typeof GRSessionScheduler.tick === "function") {
            try {
              GRSessionScheduler.tick();
            } catch (eT) {}
          }
          arm("interval_maximize_probe");
        } catch (e) {}
      }, 45000);
    } catch (e) {}
    global.GRProbeLifecycle._armReprobe = arm;
  }

  /** Server open/result said incomplete → force need_probe (cold→hot rearm). */
  function onServerProbeStatus(cps) {
    cps = cps || {};
    var incomplete =
      cps.brain_schedule_final !== true &&
      cps.final_analysis_ok !== true &&
      cps.cycle_status !== "complete";
    var force =
      cps.force_identity_probe === true ||
      cps.need_hard_anchor === true ||
      cps.cold_to_hot === true ||
      cps.accepts_identity_ingest === true;
    if (incomplete && force) {
      markNeedProbe(cps.reprobe_reason || "server_incomplete");
      markProbing();
      try {
        global.__GR_FORCE_REPROBE__ = true;
        if (global.GRProbeLifecycle && global.GRProbeLifecycle._armReprobe) {
          global.GRProbeLifecycle._armReprobe(cps.reprobe_reason || "server_incomplete");
        }
      } catch (e) {}
      return true;
    }
    return false;
  }

  global.GRProbeLifecycle = {
    STATE: STATE,
    POLICY: POLICY,
    applyServerPolicy: applyServerPolicy,
    isHardBatch: isHardBatch,
    isDeepenBatch: isDeepenBatch,
    isHeavyBatch: isHeavyBatch,
    isCommercialLandFast: isCommercialLandFast,
    isRpaBatch: isRpaBatch,
    maxAttemptsFor: maxAttemptsFor,
    heavyMaxInflight: heavyMaxInflight,
    backoffMsFor: backoffMsFor,
    recordFail: recordFail,
    softQuietActive: softQuietActive,
    rpaQuietActive: rpaQuietActive,
    deepenQuietActive: deepenQuietActive,
    allowEnqueue: allowEnqueue,
    markProbing: markProbing,
    markCool: markCool,
    markNeedProbe: markNeedProbe,
    deriveFromOpen: deriveFromOpen,
    snapshot: snapshot,
    installReprobeTriggers: installReprobeTriggers,
    installComputePressure: installComputePressure,
    timingProbesAllowed: timingProbesAllowed,
    onServerProbeStatus: onServerProbeStatus,
    getState: function () {
      return state;
    },
  };

  // Auto-install multi-path re-probe hooks as soon as lifecycle loads.
  try {
    installReprobeTriggers(null);
  } catch (e0) {}
  try {
    installComputePressure();
  } catch (e1) {}
})(typeof window !== "undefined" ? window : globalThis);
