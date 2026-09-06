/**
 * GR SessionScheduler — single arm/kick arbiter (iss/72 complete form).
 *
 * All lifecycle / hard-SLA / route-plan / multi-tick re-arms submit events here.
 * Scheduler decides: debounce, busy, cool/terminal, plan_epoch freshness, then kick.
 */
(function (global) {
  "use strict";

  var sessionId = "";
  var planEpoch = 0;
  var lastArmMs = 0;
  var pending = null; // { reason, epoch, force }
  var debounceMs = 5000;
  var handlers = {
    kick: null,
    onEvent: null,
  };
  var stats = {
    events: 0,
    kicked: 0,
    deferred_busy: 0,
    deferred_debounce: 0,
    dropped_stale_epoch: 0,
    dropped_cool: 0,
  };

  function now() {
    return Date.now();
  }

  function isCool() {
    try {
      if (global.__GR_BRAIN_SCHEDULE_FINAL__ || global.__GR_HARD_FINAL__) return true;
      if (global.__GR_HALT_UPLOADS__ && global.__GR_SKIP_IDENTITY__) return true;
      if (global.GRProbeLifecycle && GRProbeLifecycle.getState) {
        var st = GRProbeLifecycle.getState();
        if (st === "complete_cool" || st === "idle_cool") return true;
      }
    } catch (e) {}
    return false;
  }

  function uploadIsBusy() {
    try {
      if (global.GRUploadQueue && GRUploadQueue.stats) {
        var s = GRUploadQueue.stats() || {};
        if ((s.pending || 0) > 0 || (s.inflight || 0) > 0) return true;
      }
    } catch (e) {}
    return false;
  }

  // Uploading is intentionally not a global collection lock. Captures are
  // frozen before enqueue, so an acknowledged/sealed transport can overlap
  // the next value-ranked collection kick. The collection arbiter remains
  // busy only while a probe cycle is actively executing.
  function isBusy() {
    try {
      return !!global.__GR_MULTI_TICK_ACTIVE__;
    } catch (e) {
      return false;
    }
  }

  function setSession(sid) {
    if (sid && String(sid) !== String(sessionId || "")) {
      sessionId = String(sid);
      planEpoch = 0;
      pending = null;
    } else if (sid) {
      sessionId = String(sid);
    }
  }

  function setPlanEpoch(ep) {
    var n = Number(ep);
    if (!isFinite(n) || n < 0) return planEpoch;
    if (n > planEpoch) planEpoch = n;
    return planEpoch;
  }

  function getPlanEpoch() {
    return planEpoch;
  }

  /**
   * Submit a scheduler event. Does not kick immediately when busy/debounced.
   * @param {string} reason
   * @param {object} [opts] { force, plan_epoch, kick }
   */
  function submit(reason, opts) {
    opts = opts || {};
    stats.events++;
    var force = !!opts.force;
    var ep = opts.plan_epoch != null ? Number(opts.plan_epoch) : null;

    // Stale plan: client tries to act on older epoch than known.
    if (ep != null && isFinite(ep) && ep > 0 && ep < planEpoch && !force) {
      stats.dropped_stale_epoch++;
      try {
        if (handlers.onEvent) handlers.onEvent("stale_epoch", { reason: reason, ep: ep, planEpoch: planEpoch });
      } catch (e) {}
      return { ok: false, reason: "stale_plan_epoch", plan_epoch: planEpoch };
    }
    if (ep != null && isFinite(ep) && ep > planEpoch) {
      planEpoch = ep;
    }

    if (isCool() && !force && !global.__GR_FORCE_REPROBE__) {
      stats.dropped_cool++;
      return { ok: false, reason: "cool" };
    }

    var t = now();
    if (!force && (isBusy() || (opts.wait_for_upload && uploadIsBusy()))) {
      pending = { reason: reason || "busy", epoch: planEpoch, force: false, at: t };
      stats.deferred_busy++;
      return { ok: false, reason: "busy", pending: true };
    }
    if (!force && t - lastArmMs < debounceMs && reason !== "server_incomplete" && reason !== "route_plan") {
      pending = { reason: reason || "debounce", epoch: planEpoch, force: false, at: t };
      stats.deferred_debounce++;
      return { ok: false, reason: "debounce", pending: true };
    }

    return execute(reason || "submit", force);
  }

  function execute(reason, force) {
    lastArmMs = now();
    pending = null;
    stats.kicked++;
    try {
      if (global.GRProbeLifecycle) {
        if (GRProbeLifecycle.markNeedProbe) GRProbeLifecycle.markNeedProbe(reason);
        if (GRProbeLifecycle.markProbing) GRProbeLifecycle.markProbing();
      }
    } catch (eM) {}
    try {
      if (typeof handlers.kick === "function") {
        handlers.kick(reason, { force: !!force, plan_epoch: planEpoch, session_id: sessionId });
      } else if (typeof global.grResumeProbeCycle === "function") {
        global.grResumeProbeCycle(reason);
      } else if (typeof global.grKickRoutePlan === "function") {
        global.grKickRoutePlan(reason);
      } else if (global.GRBoot && typeof global.GRBoot.resumeProbe === "function") {
        global.GRBoot.resumeProbe(reason);
      }
    } catch (eK) {}
    return { ok: true, reason: reason, plan_epoch: planEpoch };
  }

  /** Flush deferred event if idle. */
  function tick() {
    if (!pending) return null;
    if (isCool() && !pending.force) {
      pending = null;
      return null;
    }
    if (isBusy()) return null;
    var p = pending;
    pending = null;
    return execute(p.reason, p.force);
  }

  /**
   * Apply route_plan only if epoch is fresh.
   * @returns {{ok:boolean, reason?:string, plan_epoch?:number}}
   */
  function acceptRoutePlan(plan, analysisRev) {
    plan = plan || {};
    var ep =
      plan.plan_epoch != null
        ? Number(plan.plan_epoch)
        : plan.plan_version != null
          ? Number(plan.plan_version)
          : analysisRev != null
            ? Number(analysisRev)
            : 0;
    if (!isFinite(ep) || ep <= 0) {
      // No epoch — accept but don't regress
      return { ok: true, plan_epoch: planEpoch, reason: "no_epoch" };
    }
    if (ep < planEpoch) {
      stats.dropped_stale_epoch++;
      return { ok: false, reason: "stale_plan_epoch", plan_epoch: planEpoch, got: ep };
    }
    planEpoch = ep;
    try {
      global.__GR_PLAN_EPOCH__ = planEpoch;
    } catch (e) {}
    return { ok: true, plan_epoch: planEpoch };
  }

  function bind(opts) {
    opts = opts || {};
    if (typeof opts.kick === "function") handlers.kick = opts.kick;
    if (typeof opts.onEvent === "function") handlers.onEvent = opts.onEvent;
    if (opts.debounce_ms != null) debounceMs = Math.max(0, Number(opts.debounce_ms) || 0);
    if (opts.session_id) setSession(opts.session_id);
  }

  function snapshot() {
    return {
      session_id: sessionId,
      plan_epoch: planEpoch,
      pending: pending,
      debounce_ms: debounceMs,
      busy: isBusy(),
      upload_busy: uploadIsBusy(),
      cool: isCool(),
      stats: {
        events: stats.events,
        kicked: stats.kicked,
        deferred_busy: stats.deferred_busy,
        deferred_debounce: stats.deferred_debounce,
        dropped_stale_epoch: stats.dropped_stale_epoch,
        dropped_cool: stats.dropped_cool,
      },
    };
  }

  // Drain pending every few seconds
  try {
    setInterval(function () {
      try {
        tick();
      } catch (e) {}
    }, 2000);
  } catch (eI) {}

  global.GRSessionScheduler = {
    bind: bind,
    setSession: setSession,
    setPlanEpoch: setPlanEpoch,
    getPlanEpoch: getPlanEpoch,
    submit: submit,
    tick: tick,
    acceptRoutePlan: acceptRoutePlan,
    isBusy: isBusy,
    uploadIsBusy: uploadIsBusy,
    isCool: isCool,
    snapshot: snapshot,
  };
})(typeof window !== "undefined" ? window : globalThis);
