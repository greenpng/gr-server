 /* greenpng race pack | 1.0.5 */

/* ---- gr.fe_impl.js ---- */
/**
 * FE implementation identity — distinct from server product_version.
 * Build injects window.__GR_BUILD_IMPL__ (= root VERSION) into each ship artifact.
 * Runtime records which modules actually loaded so every batch can stamp fe_impl_*.
 *
 * Performance: O(1) memory writes; no network; upload adds a few short strings only.
 */
(function (global) {
  "use strict";
  function buildImpl() {
    try {
      if (global.__GR_BUILD_IMPL__) return String(global.__GR_BUILD_IMPL__);
    } catch (e0) {}
    return "";
  }
  function ensure() {
    try {
      if (!global.__GR_FE_IMPL__ || typeof global.__GR_FE_IMPL__ !== "object") {
        global.__GR_FE_IMPL__ = Object.create(null);
      }
      return global.__GR_FE_IMPL__;
    } catch (e1) {
      return Object.create(null);
    }
  }
  /** Record that module `name` executed from build `ver` (default: this file's build). */
  function noteModule(name, ver) {
    try {
      var m = ensure();
      var v = String(ver || buildImpl() || "");
      if (name) m[String(name)] = v;
      if (v) {
        if (!m.build) m.build = v;
        // Prefer strongest content-proven modules for "fe_impl_version"
        if (name === "lite" || name === "hard" || name === "loader" || name === "entry") {
          if (name === "lite" || name === "hard") m.content = v;
        }
      }
      if (v && !global.__GR_FE_IMPL_VERSION__) {
        global.__GR_FE_IMPL_VERSION__ = v;
      }
      if (name === "lite" && v) {
        global.__GR_FE_LITE_IMPL__ = v;
      }
      if (name === "hard" && v) {
        global.__GR_FE_HARD_IMPL__ = v;
      }
    } catch (e2) {}
  }
  /** Server product epoch (grant / inject) — must match seal allowlist. */
  function serverProduct() {
    try {
      if (global.__GR_SERVER_PRODUCT_VERSION__)
        return String(global.__GR_SERVER_PRODUCT_VERSION__);
    } catch (eS0) {}
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (eS1) {}
    try {
      var b = global.__GR_BOOT__ || {};
      if (b.product_version) return String(b.product_version);
      if (b.version) return String(b.version);
    } catch (eS2) {}
    return "";
  }

  /** Snapshot for upload stamping (shallow). */
  function snapshot() {
    var m = ensure();
    var build = buildImpl() || m.build || "";
    var content = m.content || m.lite || m.hard || "";
    var loader = m.loader || "";
    var entry = m.entry || "";
    var product = serverProduct();
    // Seal gate binds fe_impl_version to product epoch. Prefer server product so
    // sticky/old module BUILD_IMPL stamps (v5.8.*) never reject B10 in lab/prod.
    // Content/build remain diagnostic side fields.
    var effective = product || content || build || "";
    try {
      if (effective) global.__GR_FE_IMPL_VERSION__ = effective;
    } catch (eEff) {}
    return {
      build: build,
      loader: loader,
      entry: entry,
      lite: m.lite || "",
      hard: m.hard || "",
      content: content,
      product: product,
      fe_impl_version: effective,
      fe_loader_impl: loader || build,
      fe_entry_impl: entry || "",
      fe_lite_impl: m.lite || "",
      fe_hard_impl: m.hard || "",
    };
  }
  function markContentProven(ver) {
    try {
      var v = String(ver || buildImpl() || "");
      var m = ensure();
      if (v) {
        m.content = v;
        m.lite = m.lite || v;
        m.hard = m.hard || v;
        global.__GR_FE_IMPL_VERSION__ = v;
        global.__GR_FE_PACKS_VERSION__ = v;
        global.__GR_FE_CODE_VERSION__ = v;
      }
    } catch (e3) {}
  }
  global.GRFeImpl = {
    buildImpl: buildImpl,
    noteModule: noteModule,
    snapshot: snapshot,
    markContentProven: markContentProven,
    ensure: ensure,
  };
  // Self-register if this file carries a build stamp.
  try {
    if (buildImpl()) noteModule("fe_impl_lib", buildImpl());
  } catch (e4) {}
})(typeof window !== "undefined" ? window : globalThis);

/* ---- gr.privacy_guard.js ---- */
/**
 * V5 silent-probe privacy guard.
 *
 * Product redline: the default install NEVER requests user permission
 * (camera / mic / geolocation / clipboard / notifications request / display /
 * Local Font Access queryLocalFonts / window management getScreenDetails /
 * Web MIDI requestMIDIAccess / screen wake lock / storage.persist).
 * Site business code may open mic/camera — we only OBSERVE that passively.
 *
 * Install early (boot). Reports:
 *  - blocked_n: our code tried a banned API (should be 0)
 *  - site_*: business page used sensitive APIs (allowed observation)
 * Flush into probe fields via B28 / ops report.
 */
(function (global) {
  "use strict";
  if (global.__GR_PRIVACY_GUARD__) return;
  global.__GR_PRIVACY_GUARD__ = true;

  var MAX_EVENTS = 40;
  var events = [];
  var blocked_n = 0;
  var site_getUserMedia_n = 0;
  var site_geo_n = 0;
  var site_clip_n = 0;
  var site_notify_req_n = 0;
  var site_media_kinds = { audio: 0, video: 0, display: 0, other: 0 };
  var installed = false;

  function now() {
    return Date.now();
  }

  function stackHint() {
    try {
      var s = new Error().stack || "";
      return String(s).split("\n").slice(0, 6).join(" | ").slice(0, 400);
    } catch (e) {
      return "";
    }
  }

  /** True if call looks like V5 probe collector code (not host page / Playwright). */
  function isGrStack() {
    try {
      var s = new Error().stack || "";
      // Drop privacy_guard frames — wrapper itself would otherwise always match.
      var body = String(s)
        .split("\n")
        .filter(function (ln) {
          return !/privacy_guard|GRPrivacyGuard/i.test(ln);
        })
        .join("\n");
      return /GRCollectors|registry\.(mid|static|dense)|pack_loader|probe_lifecycle|collectors\/l1|collectors\/registry/i.test(
        body
      );
    } catch (e) {
      return false;
    }
  }

  function pushEvent(kind, detail) {
    var row = {
      t: now(),
      kind: kind,
      detail: detail || {},
      from_gr: isGrStack(),
    };
    events.push(row);
    if (events.length > MAX_EVENTS) events.shift();
    try {
      if (global.GROps && typeof GROps.report === "function") {
        GROps.report("privacy_guard", kind, row, kind.indexOf("blocked") >= 0 ? "warn" : "info");
      }
    } catch (eOp) {}
    try {
      global.__GR_PRIVACY_EVENTS__ = events.slice();
    } catch (e2) {}
  }

  function banResult(name) {
    blocked_n++;
    pushEvent("blocked_" + name, { stack: stackHint() });
    var err = new Error("gr_privacy_guard_blocked:" + name);
    err.name = "NotAllowedError";
    return Promise.reject(err);
  }

  function wrapMediaDevices() {
    try {
      var md = navigator.mediaDevices;
      if (!md) return;
      if (typeof md.getUserMedia === "function" && !md.__gr_gum_wrapped) {
        var gum = md.getUserMedia.bind(md);
        md.getUserMedia = function (constraints) {
          if (isGrStack()) {
            return banResult("getUserMedia");
          }
          site_getUserMedia_n++;
          var c = constraints || {};
          if (c.audio) site_media_kinds.audio++;
          if (c.video) site_media_kinds.video++;
          if (!c.audio && !c.video) site_media_kinds.other++;
          pushEvent("site_getUserMedia", {
            has_audio: !!c.audio,
            has_video: !!c.video,
          });
          return gum(constraints);
        };
        md.__gr_gum_wrapped = true;
      }
      if (typeof md.getDisplayMedia === "function" && !md.__gr_gdm_wrapped) {
        var gdm = md.getDisplayMedia.bind(md);
        md.getDisplayMedia = function (constraints) {
          if (isGrStack()) {
            return banResult("getDisplayMedia");
          }
          site_media_kinds.display++;
          pushEvent("site_getDisplayMedia", {});
          return gdm(constraints);
        };
        md.__gr_gdm_wrapped = true;
      }
    } catch (e) {}
  }

  function wrapGeolocation() {
    try {
      var geo = navigator.geolocation;
      if (!geo) return;
      ["getCurrentPosition", "watchPosition"].forEach(function (fn) {
        if (typeof geo[fn] !== "function" || geo["__gr_" + fn]) return;
        var orig = geo[fn].bind(geo);
        geo[fn] = function () {
          if (isGrStack()) {
            blocked_n++;
            pushEvent("blocked_" + fn, { stack: stackHint() });
            var cb = arguments[1];
            if (typeof cb === "function") {
              try {
                cb({ code: 1, message: "gr_privacy_guard_blocked", PERMISSION_DENIED: 1 });
              } catch (e) {}
            }
            return fn === "watchPosition" ? -1 : undefined;
          }
          site_geo_n++;
          pushEvent("site_" + fn, {});
          return orig.apply(geo, arguments);
        };
        geo["__gr_" + fn] = true;
      });
    } catch (e) {}
  }

  function wrapNotification() {
    try {
      if (typeof Notification === "undefined") return;
      if (typeof Notification.requestPermission === "function" && !Notification.__gr_rp) {
        var rp = Notification.requestPermission.bind(Notification);
        Notification.requestPermission = function () {
          if (isGrStack()) {
            return banResult("Notification.requestPermission");
          }
          site_notify_req_n++;
          pushEvent("site_Notification_requestPermission", {});
          return rp.apply(Notification, arguments);
        };
        Notification.__gr_rp = true;
      }
    } catch (e) {}
  }

  function wrapClipboard() {
    try {
      var clip = navigator.clipboard;
      if (!clip) return;
      ["read", "readText"].forEach(function (fn) {
        if (typeof clip[fn] !== "function" || clip["__gr_" + fn]) return;
        var orig = clip[fn].bind(clip);
        clip[fn] = function () {
          if (isGrStack()) {
            return banResult("clipboard." + fn);
          }
          site_clip_n++;
          pushEvent("site_clipboard_" + fn, {});
          return orig.apply(clip, arguments);
        };
        clip["__gr_" + fn] = true;
      });
    } catch (e) {}
  }

  function wrapDeviceOrientationPermission() {
    try {
      if (
        typeof DeviceOrientationEvent !== "undefined" &&
        typeof DeviceOrientationEvent.requestPermission === "function" &&
        !DeviceOrientationEvent.__gr_rp
      ) {
        var o = DeviceOrientationEvent.requestPermission.bind(DeviceOrientationEvent);
        DeviceOrientationEvent.requestPermission = function () {
          if (isGrStack()) {
            return banResult("DeviceOrientationEvent.requestPermission");
          }
          pushEvent("site_DeviceOrientation_requestPermission", {});
          return o();
        };
        DeviceOrientationEvent.__gr_rp = true;
      }
      if (
        typeof DeviceMotionEvent !== "undefined" &&
        typeof DeviceMotionEvent.requestPermission === "function" &&
        !DeviceMotionEvent.__gr_rp
      ) {
        var m = DeviceMotionEvent.requestPermission.bind(DeviceMotionEvent);
        DeviceMotionEvent.requestPermission = function () {
          if (isGrStack()) {
            return banResult("DeviceMotionEvent.requestPermission");
          }
          pushEvent("site_DeviceMotion_requestPermission", {});
          return m();
        };
        DeviceMotionEvent.__gr_rp = true;
      }
    } catch (e) {}
  }

  /**
   * Local Font Access API — calling queryLocalFonts() pops a permission dialog
   * (Chromium / Opera often show blank/empty chrome UI). Block for GR stacks only.
   */
  function wrapQueryLocalFonts() {
    try {
      if (typeof global.queryLocalFonts !== "function" || global.__gr_qlf_wrapped) return;
      var orig = global.queryLocalFonts.bind(global);
      global.queryLocalFonts = function () {
        if (isGrStack()) {
          return banResult("queryLocalFonts");
        }
        pushEvent("site_queryLocalFonts", {});
        return orig.apply(global, arguments);
      };
      global.__gr_qlf_wrapped = true;
    } catch (e) {}
  }

  /**
   * Window Management API — getScreenDetails() pops the screen-management
   * permission dialog. Block for GR stacks only (site code passes through).
   */
  function wrapGetScreenDetails() {
    try {
      if (typeof global.getScreenDetails !== "function" || global.__gr_gsd_wrapped) return;
      var orig = global.getScreenDetails.bind(global);
      global.getScreenDetails = function () {
        if (isGrStack()) {
          return banResult("getScreenDetails");
        }
        pushEvent("site_getScreenDetails", {});
        return orig.apply(global, arguments);
      };
      global.__gr_gsd_wrapped = true;
    } catch (e) {}
  }

  /** Web MIDI API — requestMIDIAccess() pops the MIDI devices permission dialog. */
  function wrapMidiAccess() {
    try {
      if (typeof navigator.requestMIDIAccess !== "function" || navigator.__gr_midi_wrapped) return;
      var orig = navigator.requestMIDIAccess.bind(navigator);
      navigator.requestMIDIAccess = function () {
        if (isGrStack()) {
          return banResult("requestMIDIAccess");
        }
        pushEvent("site_requestMIDIAccess", {});
        return orig.apply(navigator, arguments);
      };
      navigator.__gr_midi_wrapped = true;
    } catch (e) {}
  }

  /** Screen Wake Lock — wakeLock.request(type) pops a wake-lock prompt (Chromium). */
  function wrapWakeLock() {
    try {
      var wl = navigator.wakeLock;
      if (!wl || typeof wl.request !== "function" || wl.__gr_wl_wrapped) return;
      var orig = wl.request.bind(wl);
      wl.request = function (type) {
        if (isGrStack()) {
          return banResult("wakeLock.request");
        }
        pushEvent("site_wakeLock_request", { type: String(type) });
        return orig.apply(wl, arguments);
      };
      wl.__gr_wl_wrapped = true;
    } catch (e) {}
  }

  /**
   * storage.persist() is a user-visible permission-style request (install /
   * persistent storage). Block for GR stacks only; estimate() / persisted()
   * stay open — readonly probes rely on them.
   */
  function wrapStoragePersist() {
    try {
      var st = navigator.storage;
      if (!st || typeof st.persist !== "function" || st.__gr_sp_wrapped) return;
      var orig = st.persist.bind(st);
      st.persist = function () {
        if (isGrStack()) {
          return banResult("storage.persist");
        }
        pushEvent("site_storage_persist", {});
        return orig.apply(st, arguments);
      };
      st.__gr_sp_wrapped = true;
    } catch (e) {}
  }

  function install() {
    // Always (re)apply wraps — mediaDevices may appear late (Safari/WebKit).
    wrapMediaDevices();
    wrapGeolocation();
    wrapNotification();
    wrapClipboard();
    wrapDeviceOrientationPermission();
    wrapQueryLocalFonts();
    wrapGetScreenDetails();
    wrapMidiAccess();
    wrapWakeLock();
    wrapStoragePersist();
    if (!installed) {
      installed = true;
      pushEvent("guard_installed", { policy: "silent_no_permission_request" });
    }
    return snapshot();
  }

  function snapshot() {
    return {
      algo: "gr_privacy_guard_v1",
      installed: installed,
      blocked_n: blocked_n,
      site_getUserMedia_n: site_getUserMedia_n,
      site_geo_n: site_geo_n,
      site_clip_n: site_clip_n,
      site_notify_req_n: site_notify_req_n,
      site_media_kinds: {
        audio: site_media_kinds.audio,
        video: site_media_kinds.video,
        display: site_media_kinds.display,
        other: site_media_kinds.other,
      },
      site_media_access_observed: site_getUserMedia_n > 0,
      events_n: events.length,
      events_tail: events.slice(-12),
      policy: "silent_no_permission_request",
      banned_for_probe: [
        "getUserMedia",
        "getDisplayMedia",
        "geolocation.getCurrentPosition",
        "geolocation.watchPosition",
        "Notification.requestPermission",
        "clipboard.read*",
        "DeviceOrientation/Motion.requestPermission",
        "queryLocalFonts",
        "getScreenDetails",
        "requestMIDIAccess",
        "wakeLock.request",
        "storage.persist",
      ],
      allowed_passive: [
        "enumerateDevices_counts_only",
        "Notification.permission_read",
        "observe_site_getUserMedia",
        "WebRTC_no_media_tracks",
        "OfflineAudioContext",
        "fontPresence_measureText_only",
        "queryLocalFonts_typeof_only",
        "storage_estimate_persisted_readonly",
      ],
    };
  }

  function flushFields() {
    var s = snapshot();
    return {
      privacy_guard_algo: s.algo,
      privacy_policy: s.policy,
      privacy_guard_blocked_n: s.blocked_n,
      privacy_guard_installed: s.installed,
      site_media_access_observed: s.site_media_access_observed,
      site_getUserMedia_n: s.site_getUserMedia_n,
      site_geo_n: s.site_geo_n,
      site_clip_n: s.site_clip_n,
      site_notify_req_n: s.site_notify_req_n,
      site_media_kinds: s.site_media_kinds,
      privacy_guard_events_n: s.events_n,
    };
  }

  global.GRPrivacyGuard = {
    install: install,
    snapshot: snapshot,
    flushFields: flushFields,
    events: function () {
      return events.slice();
    },
  };

  // Auto-install as early as possible
  try {
    install();
  } catch (eI) {}
})(typeof window !== "undefined" ? window : globalThis);

/* ---- probe_lifecycle.js ---- */
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

/* ---- session_scheduler.js ---- */
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

/* ---- origin_coordinator.js ---- */
/**
 * GR Origin Coordinator — same-origin multi-tab heavy-role intelligence.
 *
 * Scope: ONE browser origin (shop.example.com tabs share this; news.example.com is separate).
 *
 * Roles:
 *   heavy     — may run B10 / hard silicon collect
 *   light     — L1 + RPA + transport only (no heavy re-collect)
 *   transport — flush queue / gap-fill uploads only
 *
 * Mechanisms:
 *   1) navigator.locks (preferred) for atomic heavy lease
 *   2) localStorage lease + heartbeat (fallback + cross-tab visibility)
 *   3) BroadcastChannel for progress / handoff / cool (immediate, best-effort)
 *   4) Durable handoff ticket in localStorage (survives closing tab; storage event
 *      + poll claim) — industry pattern: BC alone drops if no live listener;
 *      GA4/Sentry use beacon for payloads; task *continuation* needs shared durable state
 *      (RxDB leader + IDB; we use LS ticket + Web Locks for lighter footprint).
 *
 * Handoff:
 *   - pagehide/beforeunload: release heavy + write durable unfinished ticket + BC
 *   - survivors (or same-tab multipage nav resume): claim ticket → upgrade heavy → ensureGap
 *   - clear ticket when server has B10 / work satisfied
 *
 * @see reports/FE_PROBE_GLOBAL_INTELLIGENCE_ARCH_V1.md
 * @see reports/MULTI_TAB_HANDOFF_RESEARCH_V1.md
 */
(function (global) {
  "use strict";

  if (global.GROriginCoordinator && global.GROriginCoordinator.__ready) return;

  var CH_NAME = "gr-gpi-v1";
  var LS_LEASE = "_g5_gpi_lease";
  var LS_PROGRESS = "_g5_gpi_prog";
  var LS_HANDOFF = "_g5_gpi_handoff";
  var LS_PEERS = "_g5_gpi_peers";
  var LOCK_NAME = "gr-heavy-b10";
  var HEARTBEAT_MS = 2000;
  var LEASE_TTL_MS = 6000;
  var UPGRADE_AFTER_MS = 10000;
  var PROGRESS_STALE_MS = 12000;
  var HANDOFF_TTL_MS = 120000; // 2 min — claim window after tab close / route change
  var HANDOFF_CLAIM_STALE_MS = 15000; // re-claim if claimer died mid-upgrade
  var PEER_TTL_MS = 10000;

  var tabId = null;
  var role = "light";
  var started = false;
  var bc = null;
  var hbTimer = null;
  var watchTimer = null;
  var lockHeld = false;
  var lockRelease = null;
  var cycleId = "";
  var claimingHandoff = false;
  var stats = {
    becomes_heavy: 0,
    becomes_light: 0,
    handoffs_sent: 0,
    handoffs_recv: 0,
    handoffs_durable_write: 0,
    handoffs_claim: 0,
    upload_handoffs: 0,
    upgrades: 0,
    heartbeats: 0,
    peer_max: 1,
  };

  function now() {
    return Date.now();
  }

  function mintTabId() {
    try {
      var k = "__gr_tab_win_id__";
      var ss = global.sessionStorage;
      if (ss) {
        var ex = ss.getItem(k);
        if (ex) return ex;
        var id = "t" + Math.random().toString(36).slice(2, 10) + now().toString(36).slice(-4);
        ss.setItem(k, id);
        return id;
      }
    } catch (e) {}
    return "t" + Math.random().toString(36).slice(2, 12);
  }

  function debugOn() {
    try {
      return !!(
        global.__GR_DEBUG_GPI__ ||
        global.__GR_DEBUG__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.debug_gpi)
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
      var line = "[gr:gpi] " + msg;
      if (detail != null) fn.call(global.console, line, detail);
      else fn.call(global.console, line);
    } catch (e) {}
  }

  function ops(code, detail, sev) {
    try {
      if (global.GROps && GROps.report) {
        GROps.report(code, "origin_coord", detail || {}, sev || "info");
      }
    } catch (e) {}
    clog(sev === "warn" || sev === "error" ? "warn" : "info", code, detail || null);
  }

  function timeline(ev, d) {
    try {
      var tl = global.__GR_PROBE_TIMELINE__;
      if (!Array.isArray(tl)) {
        tl = [];
        global.__GR_PROBE_TIMELINE__ = tl;
      }
      tl.push({ t: now(), ev: "gpi_" + ev, d: d || null });
      if (tl.length > 100) tl.splice(0, tl.length - 100);
    } catch (e) {}
    clog("debug", "timeline " + ev, d || null);
  }

  function readJson(key) {
    try {
      var s = global.localStorage && localStorage.getItem(key);
      if (!s) return null;
      return JSON.parse(s);
    } catch (e) {
      return null;
    }
  }

  function writeJson(key, obj) {
    try {
      if (global.localStorage) localStorage.setItem(key, JSON.stringify(obj));
    } catch (e) {}
  }

  function clearKey(key) {
    try {
      if (global.localStorage) localStorage.removeItem(key);
    } catch (e) {}
  }

  function sid() {
    try {
      return String(
        cycleId ||
          global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.GRUploadQueue && GRUploadQueue.cfg && GRUploadQueue.cfg.session_id) ||
          ""
      );
    } catch (e) {
      return cycleId || "";
    }
  }

  function prunePeers(map) {
    var out = map && typeof map === "object" ? map : {};
    var t = now();
    Object.keys(out).forEach(function (k) {
      var p = out[k];
      if (!p || t - Number(p.hb || 0) > PEER_TTL_MS) delete out[k];
    });
    return out;
  }

  function touchPeer() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    all[tabId] = {
      hb: now(),
      role: role,
      cycle: sid(),
      href: (function () {
        try {
          return String(location.pathname || "").slice(0, 80);
        } catch (e) {
          return "";
        }
      })(),
    };
    writeJson(LS_PEERS, all);
    var n = Object.keys(all).length;
    if (n > stats.peer_max) stats.peer_max = n;
    return n;
  }

  function dropPeer() {
    try {
      var all = prunePeers(readJson(LS_PEERS) || {});
      delete all[tabId];
      writeJson(LS_PEERS, all);
    } catch (e) {}
  }

  function peerCount() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    return Object.keys(all).length || (started ? 1 : 0);
  }

  /**
   * Multi-tab brain policy — consumed by self-heal / pack_loader / upload.
   * More tabs ⇒ focus heavy on silicon; light tabs = transport + RPA only.
   */
  function policySnapshot() {
    var n = peerCount();
    var multi = n > 1;
    return {
      peer_count: n,
      multi_tab: multi,
      role: role,
      heavy: role === "heavy",
      // light tabs: transport-first (PostHog/Sentry-style queue drain)
      transport_assist: multi && role !== "heavy",
      // heavy under multi-tab: serialize silicon, less deepen thrash
      serial_heavy: multi && role === "heavy",
      // allow light to skip hard collect (pack_loader already gates)
      defer_heavy_collect: multi && role !== "heavy",
      // slightly faster transport nudge when peers help
      transport_priority: multi ? (role === "heavy" ? 1 : 2) : 1,
    };
  }

  function publishRole() {
    try {
      var pol = policySnapshot();
      global.__GR_GPI_ROLE__ = role;
      global.__GR_GPI_TAB_ID__ = tabId;
      global.__GR_GPI_HEAVY__ = role === "heavy";
      global.__GR_GPI_PEER_COUNT__ = pol.peer_count;
      global.__GR_GPI_POLICY__ = pol;
      global.__GR_GPI_MULTI_TAB__ = !!pol.multi_tab;
    } catch (e) {}
  }

  function setRole(next, why) {
    next = String(next || "light");
    if (next !== "heavy" && next !== "light" && next !== "transport") next = "light";
    if (role === next) {
      publishRole();
      return role;
    }
    var prev = role;
    role = next;
    publishRole();
    if (next === "heavy") stats.becomes_heavy++;
    else stats.becomes_light++;
    timeline("role", { from: prev, to: next, why: why || "" });
    ops(
      "gpi_role",
      { tab: tabId, from: prev, to: next, why: why || "", cycle: sid().slice(0, 28) },
      "info"
    );
    broadcast({ type: "role", tab: tabId, role: role, cycle: sid(), why: why || "" });
    // Nudge self-heal to re-evaluate collect rights immediately.
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
    } catch (eT) {}
    return role;
  }

  function broadcast(msg) {
    try {
      if (bc) bc.postMessage(Object.assign({ t: now(), from: tabId }, msg || {}));
    } catch (e) {}
  }

  function leaseSnapshot() {
    return readJson(LS_LEASE);
  }

  function leaseIsMine(L) {
    return !!(L && L.tab === tabId);
  }

  function leaseAlive(L) {
    if (!L || !L.tab) return false;
    var hb = Number(L.hb || L.at || 0);
    return now() - hb < LEASE_TTL_MS;
  }

  function writeLease(extra) {
    var L = Object.assign(
      {
        tab: tabId,
        at: now(),
        hb: now(),
        cycle: sid(),
      },
      extra || {}
    );
    writeJson(LS_LEASE, L);
    return L;
  }

  function clearMyLease() {
    var L = leaseSnapshot();
    if (leaseIsMine(L)) clearKey(LS_LEASE);
  }

  function writeProgress(patch) {
    var p = Object.assign(
      {
        tab: tabId,
        role: role,
        cycle: sid(),
        at: now(),
        has_b10_local: false,
        phase: "",
      },
      patch || {}
    );
    try {
      if (global.__GR_RECEIVED_BATCHES__) {
        /* noop */
      }
      var st =
        global.GRUploadQueue &&
        GRUploadQueue.materialState &&
        GRUploadQueue.materialState("B10_hw_curves", sid(), "main");
      if (st === "acked") p.has_b10_local = true;
    } catch (e) {}
    try {
      if (global.__GR_CYCLE_PROBE_STATUS__ && global.__GR_CYCLE_PROBE_STATUS__.has_b10) {
        p.has_b10_server = true;
      }
    } catch (e2) {}
    writeJson(LS_PROGRESS, p);
    broadcast({ type: "progress", progress: p });
    return p;
  }

  function serverHasB10() {
    try {
      var cps = global.__GR_CYCLE_PROBE_STATUS__ || {};
      if (cps.has_b10 === true) return true;
      var cov = cps.identity_coverage || {};
      if (cov.has_b10 === true) return true;
    } catch (e) {}
    try {
      var so = global.__GR_SESSION_OUTCOME__ || {};
      if (so.batches_ok && (so.batches_ok.B10_hw_curves || so.batches_ok["mid.curves"])) return true;
    } catch (e2) {}
    return false;
  }

  function canHeavyCollect() {
    if (serverHasB10()) return false;
    if (role === "heavy") return true;
    // No live lease and we need upgrade path
    var L = leaseSnapshot();
    if (!leaseAlive(L)) return role === "heavy";
    return false;
  }

  function isHeavy() {
    return role === "heavy";
  }

  function releaseHeavy(why) {
    if (lockRelease) {
      try {
        lockRelease();
      } catch (e) {}
      lockRelease = null;
    }
    lockHeld = false;
    clearMyLease();
    if (role === "heavy") setRole("light", why || "release");
    broadcast({ type: "lease_free", why: why || "release", tab: tabId });
  }

  function becomeHeavy(why) {
    writeLease({ why: why || "acquire" });
    lockHeld = true;
    setRole("heavy", why || "acquire");
    writeProgress({ role: "heavy", phase: "heavy_start" });
    stats.becomes_heavy++;
  }

  /**
   * Try to acquire heavy role. Async; resolves with boolean.
   */
  function tryAcquireHeavy(why) {
    return new Promise(function (resolve) {
      if (serverHasB10()) {
        setRole("light", "server_has_b10");
        resolve(false);
        return;
      }
      var L0 = leaseSnapshot();
      if (leaseAlive(L0) && !leaseIsMine(L0)) {
        setRole("light", "other_leader");
        resolve(false);
        return;
      }
      // Prefer Web Locks — if busy, stay light (do NOT fall through to storage race).
      try {
        if (global.navigator && navigator.locks && typeof navigator.locks.request === "function") {
          var settled = false;
          function finish(ok, whyF) {
            if (settled) return;
            settled = true;
            if (!ok) setRole("light", whyF || "lock_busy");
            resolve(!!ok);
          }
          navigator.locks
            .request(LOCK_NAME, { ifAvailable: true }, function (lock) {
              if (!lock) {
                finish(false, "lock_busy");
                return;
              }
              // Re-check storage lease after lock grant (another tab may have storage-only heavy).
              var L1 = leaseSnapshot();
              if (leaseAlive(L1) && !leaseIsMine(L1)) {
                finish(false, "storage_peer");
                return;
              }
              lockHeld = true;
              var released = false;
              lockRelease = function () {
                if (released) return;
                released = true;
              };
              becomeHeavy(why || "web_lock");
              finish(true, "web_lock");
              return new Promise(function (holdDone) {
                var iv = setInterval(function () {
                  if (!lockHeld || role !== "heavy") {
                    clearInterval(iv);
                    holdDone();
                  }
                }, 500);
              });
            })
            .catch(function () {
              // Locks API error → storage fallback only
              if (!settled) storageAcquire(why, resolve);
            });
          return;
        }
      } catch (eL) {}
      storageAcquire(why, resolve);
    });
  }

  function storageAcquire(why, resolve) {
    // Optimistic atomic-ish: re-read, write only if free/stale/mine
    var L = leaseSnapshot();
    if (leaseAlive(L) && !leaseIsMine(L)) {
      setRole("light", "storage_busy");
      resolve(false);
      return;
    }
    // Claim with random token to detect clobber
    var token = tabId + ":" + Math.random().toString(36).slice(2, 8);
    writeLease({ why: why || "storage_lease", token: token });
    // Verify we still own after write (last-writer check)
    var L2 = leaseSnapshot();
    if (!L2 || L2.tab !== tabId) {
      setRole("light", "storage_lost_race");
      resolve(false);
      return;
    }
    becomeHeavy(why || "storage_lease");
    resolve(true);
  }

  function heartbeat() {
    stats.heartbeats++;
    touchPeer();
    publishRole();
    // Nudge self-heal adaptive when multi-tab policy changes
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.applyAdaptivePolicy) {
        GRProbeSelfHeal.applyAdaptivePolicy(true);
      }
    } catch (eP) {}
    if (role === "heavy") {
      writeLease({ hb: now(), cycle: sid() });
      writeProgress({ role: "heavy", phase: "heartbeat", peers: peerCount() });
      if (serverHasB10()) {
        var Hu = readHandoff();
        if (!Hu || !Hu.upload_pending || !Hu.upload_pending.length) clearHandoff("heavy_server_b10");
      }
    } else {
      writeProgress({ role: role, phase: "follower", peers: peerCount() });
      maybeUpgrade();
      processHandoff("heartbeat");
      // Light multi-tab: actively assist transport (queue pump)
      try {
        var pol = policySnapshot();
        if (pol.transport_assist && global.GRUploadQueue && GRUploadQueue.flush) {
          // soft background flush — not unloading
          GRUploadQueue.flush("gpi_transport_assist");
        }
      } catch (eT) {}
    }
  }

  function maybeUpgrade() {
    if (role === "heavy") return;
    if (serverHasB10()) return;
    var L = leaseSnapshot();
    var prog = readJson(LS_PROGRESS);
    // Leader dead or progress stale → upgrade
    var leaderDead = !leaseAlive(L);
    var progressStale =
      !prog || !prog.at || now() - Number(prog.at) > PROGRESS_STALE_MS || prog.tab !== (L && L.tab);
    var waited = started ? now() - (global.__GR_GPI_START_MS__ || now()) : 0;
    if (leaderDead || (progressStale && waited > UPGRADE_AFTER_MS)) {
      tryAcquireHeavy(leaderDead ? "upgrade_lease_dead" : "upgrade_stale_progress").then(function (ok) {
        if (ok) {
          stats.upgrades++;
          timeline("upgrade", { why: leaderDead ? "lease_dead" : "stale" });
          ops("gpi_upgrade", { tab: tabId, why: leaderDead ? "lease_dead" : "stale" }, "warn");
        }
      });
    }
  }

  function onMessage(ev) {
    var m = (ev && ev.data) || {};
    if (!m || m.from === tabId) return;
    if (m.type === "progress" && m.progress) {
      try {
        // Cache peer progress for diagnostics
        global.__GR_GPI_PEER__ = m.progress;
      } catch (e) {}
      if (m.progress.has_b10_local || m.progress.has_b10_server) {
        if (role === "heavy" && !serverHasB10()) {
          /* keep heavy until server confirms — avoid flip-flop */
        } else if (role === "heavy" && serverHasB10()) {
          releaseHeavy("peer_or_server_b10");
        }
      }
    }
    if (m.type === "handoff") {
      stats.handoffs_recv++;
      timeline("handoff_recv", m);
      ops(
        "gpi_handoff_recv",
        {
          from: m.from,
          unfinished: m.unfinished || [],
          cycle: m.cycle,
          durable: !!m.durable,
        },
        "warn"
      );
      // Mirror into durable store if peer only BC'd (older path / race)
      if (Array.isArray(m.unfinished) && m.unfinished.length && !serverHasB10()) {
        var cur = readHandoff();
        if (!handoffIsActionable(cur)) {
          writeJson(LS_HANDOFF, {
            v: 1,
            from: m.from,
            cycle: m.cycle || sid(),
            unfinished: m.unfinished.slice(),
            at: now(),
            why: "bc_mirror",
            claimed_by: null,
            claimed_at: 0,
          });
        }
      }
      processHandoff("bc");
    }
    if (m.type === "cool" || m.type === "has_b10") {
      if (serverHasB10()) clearHandoff("peer_has_b10");
      if (role === "heavy") {
        // Demote after peer cool if server has b10
        if (serverHasB10()) releaseHeavy("peer_cool");
      }
    }
    if (m.type === "lease_free") {
      maybeUpgrade();
      processHandoff("lease_free");
    }
  }

  function collectUnfinished() {
    var out = [];
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.snapshot) {
        var snap = GRProbeSelfHeal.snapshot();
        (snap.gaps || []).forEach(function (g) {
          if (g && g.desired && !g.satisfied && (g.batch_id === "B10_hw_curves" || g.batch_id === "mid.curves")) {
            out.push(g.batch_id);
          }
        });
      }
    } catch (e) {}
    // Also inspect pack health — heavy mid-flight may not have gap entry yet
    try {
      var packs = global.__GR_PACK_HEALTH__ || {};
      var b10h = packs.B10_hw_curves || packs["mid.curves"];
      if (b10h && (b10h.run === "running" || b10h.run === "queued" || b10h.state === "running")) {
        if (out.indexOf("B10_hw_curves") < 0) out.push("B10_hw_curves");
      }
    } catch (e2) {}
    if (!out.length && !serverHasB10() && role === "heavy") out.push("B10_hw_curves");
    // de-dup
    var seen = {};
    return out.filter(function (x) {
      if (seen[x]) return false;
      seen[x] = 1;
      return true;
    });
  }

  /**
   * Pending upload batch ids (not full payloads — sealed bodies too large for LS).
   * Survivor re-collects if server missing; otherwise transport nudge only.
   * Mirrors PostHog retry-queue persistence of *request identity*, not always body.
   */
  function collectUploadPending() {
    var out = [];
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.stats === "function") {
        var st = q.stats() || {};
        var pend = st.pending_batches || st.pending_ids || st.batches_pending || null;
        if (Array.isArray(pend)) {
          pend.forEach(function (b) {
            if (b && out.indexOf(String(b)) < 0) out.push(String(b));
          });
        }
      }
      // material state scan for hard anchors still not acked
      if (q && typeof q.materialState === "function") {
        ["B10_hw_curves", "mid.curves", "B0_bootstrap", "B2_hardware", "B3_system"].forEach(function (bid) {
          try {
            var ms = q.materialState(bid, sid(), "main");
            if (ms && ms !== "acked" && ms !== "absent" && out.indexOf(bid) < 0) {
              if (ms === "queued" || ms === "uploading" || ms === "retry_wait" || ms === "collected" || ms === "new") {
                out.push(bid);
              }
            }
          } catch (eM) {}
        });
      }
    } catch (e) {}
    return out.slice(0, 24);
  }

  function readHandoff() {
    return readJson(LS_HANDOFF);
  }

  function handoffIsActionable(H) {
    if (!H) return false;
    var at = Number(H.at || 0);
    if (!at || now() - at > HANDOFF_TTL_MS) return false;
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    var needUpload = Array.isArray(H.upload_pending) && H.upload_pending.length > 0;
    if (!needCollect && !needUpload) return false;
    // Peer already claimed and still looks alive → leave it (collect path)
    if (needCollect && H.claimed_by && H.claimed_by !== tabId) {
      var cAt = Number(H.claimed_at || 0);
      var claimFresh = cAt && now() - cAt < HANDOFF_CLAIM_STALE_MS;
      var L = leaseSnapshot();
      var claimerHoldsLease = leaseAlive(L) && L.tab === H.claimed_by;
      if (claimerHoldsLease) return false;
      if (claimFresh && leaseAlive(L) && L.tab !== tabId) return false;
    }
    return true;
  }

  /**
   * Durable ticket: localStorage survives the dying tab.
   * BC is best-effort notify for already-open survivors.
   * Same-tab multipage navigation also pagehides — ticket lets the next document resume.
   */
  function writeDurableHandoff(unfinished, why, uploadPending) {
    uploadPending = uploadPending || [];
    var needCollect = unfinished && unfinished.length && !serverHasB10();
    var needUpload = uploadPending && uploadPending.length;
    if (!needCollect && !needUpload) return null;
    var ticket = {
      v: 2,
      from: tabId,
      cycle: sid(),
      unfinished: needCollect ? unfinished.slice() : [],
      upload_pending: needUpload ? uploadPending.slice(0, 24) : [],
      at: now(),
      why: why || "pagehide",
      href: (function () {
        try {
          return String(location.href || "").slice(0, 160);
        } catch (e) {
          return "";
        }
      })(),
      role_was: role,
      claimed_by: null,
      claimed_at: 0,
    };
    // Prefer keep fresher ticket if same cycle still open
    var prev = readHandoff();
    if (prev && (handoffIsActionable(prev) || (prev.upload_pending && prev.upload_pending.length)) && prev.cycle === ticket.cycle) {
      var merged = (prev.unfinished || []).slice();
      (ticket.unfinished || []).forEach(function (u) {
        if (merged.indexOf(u) < 0) merged.push(u);
      });
      ticket.unfinished = merged;
      var um = (prev.upload_pending || []).slice();
      (ticket.upload_pending || []).forEach(function (u) {
        if (um.indexOf(u) < 0) um.push(u);
      });
      ticket.upload_pending = um.slice(0, 24);
      if (prev.claimed_by && prev.claimed_by !== tabId) {
        ticket.claimed_by = prev.claimed_by;
        ticket.claimed_at = prev.claimed_at;
      }
    }
    writeJson(LS_HANDOFF, ticket);
    stats.handoffs_durable_write++;
    if (ticket.upload_pending && ticket.upload_pending.length) stats.upload_handoffs++;
    timeline("handoff_durable", {
      unfinished: ticket.unfinished,
      upload_pending: ticket.upload_pending,
      why: why || "pagehide",
    });
    ops(
      "gpi_handoff_durable",
      {
        unfinished: ticket.unfinished,
        upload_pending: ticket.upload_pending,
        tab: tabId,
        why: why || "pagehide",
      },
      "warn"
    );
    return ticket;
  }

  function clearHandoff(why) {
    var H = readHandoff();
    if (!H) return;
    // Only clear if we own claim or ticket is ours / satisfied
    if (H.claimed_by && H.claimed_by !== tabId && handoffIsActionable(H)) return;
    clearKey(LS_HANDOFF);
    timeline("handoff_clear", { why: why || "done" });
    clog("info", "handoff_clear", { why: why || "done" });
  }

  function kickUploadAssist(uploadPending, reason) {
    if (!uploadPending || !uploadPending.length) return;
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.nudgeTransport === "function") {
        uploadPending.forEach(function (bid) {
          try {
            q.nudgeTransport(String(bid), sid());
          } catch (eN) {}
        });
      }
      if (q && typeof q.flush === "function") {
        q.flush("handoff_assist");
      }
    } catch (eF) {}
    try {
      if (global.GRProbeSelfHeal) {
        uploadPending.forEach(function (bid) {
          if (!bid) return;
          // Server missing → recollect; else transport loop will settle.
          if (GRProbeSelfHeal.ensureGap) {
            GRProbeSelfHeal.ensureGap(String(bid), "main", {
              desired: true,
              force_recollect: !serverHasB10() && (bid === "B10_hw_curves" || bid === "mid.curves"),
              reason: reason || "upload_handoff",
            });
          }
        });
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
  }

  function kickHeavyWork(reason) {
    // Clear stale light-defer markers so pack_loader will re-enter B10 after upgrade.
    try {
      var ph = global.__GR_PACK_HEALTH__;
      if (ph && typeof ph === "object") {
        ["B10_hw_curves", "mid.curves"].forEach(function (k) {
          if (ph[k] && ph[k].run === "deferred_gpi_light") {
            ph[k].run = "handoff_pending";
            ph[k].handoff_reason = reason || "handoff";
          }
        });
      }
    } catch (ePh) {}
    try {
      if (global.GRProbeSelfHeal) {
        if (GRProbeSelfHeal.ensureGap) {
          GRProbeSelfHeal.ensureGap("B10_hw_curves", "main", {
            desired: true,
            force_recollect: true,
            reason: reason || "handoff",
          });
        }
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
    try {
      var H = readHandoff();
      if (H && H.upload_pending) kickUploadAssist(H.upload_pending, reason);
    } catch (eU) {}
  }

  /**
   * Claim durable handoff and upgrade to heavy if needed.
   * Idempotent; safe to call from heartbeat / storage / BC / start.
   */
  function processHandoff(source) {
    if (claimingHandoff) return;
    var H = readHandoff();
    if (!handoffIsActionable(H)) {
      // stale cleanup
      if (H) {
        var empty =
          (!H.unfinished || !H.unfinished.length) &&
          (!H.upload_pending || !H.upload_pending.length);
        var expired = now() - Number(H.at || 0) > HANDOFF_TTL_MS;
        if (empty || expired) {
          if (!H.claimed_by || H.claimed_by === tabId) clearKey(LS_HANDOFF);
        }
      }
      if (serverHasB10() && H && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("server_has_b10");
      }
      return;
    }
    // Upload assist: any tab (light or heavy) can help drain / recollect missing
    if (H.upload_pending && H.upload_pending.length) {
      kickUploadAssist(H.upload_pending, "upload_handoff_" + (source || "poll"));
    }
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    if (!needCollect) {
      // only upload path — no heavy upgrade required
      if (serverHasB10() && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("upload_done");
      }
      return;
    }
    // If we are already heavy, just kick work and mark claim
    if (role === "heavy") {
      H.claimed_by = tabId;
      H.claimed_at = now();
      writeJson(LS_HANDOFF, H);
      kickHeavyWork("handoff_already_heavy");
      return;
    }
    claimingHandoff = true;
    // Optimistic claim stamp (helps multi-survivor race)
    H.claimed_by = tabId;
    H.claimed_at = now();
    writeJson(LS_HANDOFF, H);
    tryAcquireHeavy(source === "start" ? "handoff_resume" : "handoff").then(function (ok) {
      claimingHandoff = false;
      if (!ok) {
        // Lost race — leave ticket for winner; clear our claim if still us
        var H2 = readHandoff();
        if (H2 && H2.claimed_by === tabId) {
          H2.claimed_by = null;
          H2.claimed_at = 0;
          writeJson(LS_HANDOFF, H2);
        }
        // Still help upload as light
        kickUploadAssist((H && H.upload_pending) || [], "light_upload_assist");
        return;
      }
      stats.handoffs_claim++;
      stats.handoffs_recv++;
      timeline("handoff_claim", { source: source || "poll", unfinished: H.unfinished });
      ops(
        "gpi_handoff_claim",
        { tab: tabId, source: source || "poll", unfinished: H.unfinished },
        "warn"
      );
      kickHeavyWork("handoff_claim");
    });
  }

  function onPageHide() {
    // Best-effort flush uploads first (GA4/Sentry/PostHog pattern)
    try {
      if (global.GRUploadQueue && GRUploadQueue.flush) {
        GRUploadQueue.flush("pagehide");
      }
    } catch (eFl) {}
    var unfinished = collectUnfinished();
    var uploadPending = collectUploadPending();
    if (unfinished.length || uploadPending.length) {
      stats.handoffs_sent++;
      writeDurableHandoff(
        unfinished,
        role === "heavy" ? "pagehide_heavy" : "pagehide",
        uploadPending
      );
      broadcast({
        type: "handoff",
        unfinished: unfinished,
        upload_pending: uploadPending,
        cycle: sid(),
        role: role,
        durable: true,
      });
      timeline("handoff_send", {
        unfinished: unfinished,
        upload_pending: uploadPending,
        durable: true,
      });
      ops(
        "gpi_handoff_send",
        {
          unfinished: unfinished,
          upload_pending: uploadPending,
          tab: tabId,
          durable: true,
        },
        "warn"
      );
    }
    dropPeer();
    if (role === "heavy") releaseHeavy("pagehide");
  }

  function start(opts) {
    opts = opts || {};
    if (started) return api;
    started = true;
    tabId = mintTabId();
    try {
      global.__GR_GPI_START_MS__ = now();
    } catch (e) {}
    if (opts.cycle_id) cycleId = String(opts.cycle_id);
    try {
      cycleId = cycleId || sid();
    } catch (e2) {}

    try {
      if (typeof BroadcastChannel !== "undefined") {
        bc = new BroadcastChannel(CH_NAME);
        bc.onmessage = onMessage;
      }
    } catch (eBc) {
      bc = null;
    }

    try {
      global.addEventListener("pagehide", onPageHide);
      global.addEventListener("beforeunload", onPageHide);
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "visible") {
            heartbeat();
            maybeUpgrade();
          }
        });
      }
      global.addEventListener("storage", function (ev) {
        if (!ev) return;
        if (ev.key === LS_LEASE || ev.key === LS_PROGRESS) {
          maybeUpgrade();
        }
        if (ev.key === LS_HANDOFF) {
          processHandoff("storage");
        }
      });
    } catch (eH) {}

    // Initial role acquisition; then process any durable handoff left by a closed tab
    // or by same-tab multipage navigation (pagehide wrote ticket before unload).
    tryAcquireHeavy(opts.why || "start").then(function (ok) {
      if (!ok) setRole("light", "start_follower");
      publishRole();
      // Slight delay so self-heal/pack_loader are ready for ensureGap
      setTimeout(function () {
        processHandoff("start");
      }, 80);
    });

    hbTimer = setInterval(heartbeat, HEARTBEAT_MS);
    watchTimer = setInterval(function () {
      maybeUpgrade();
      processHandoff("watch");
      if (serverHasB10()) {
        clearHandoff("watch_server_b10");
        if (role === "heavy") {
          // Keep heavy briefly for deepen; demote after coverage stable
          writeProgress({ has_b10_server: true, phase: "b10_done" });
          broadcast({ type: "has_b10", cycle: sid() });
        }
      }
    }, 3000);

    timeline("start", { tab: tabId });
    ops("gpi_start", { tab: tabId }, "info");
    return api;
  }

  function stop(why) {
    started = false;
    if (hbTimer) {
      try {
        clearInterval(hbTimer);
      } catch (e) {}
      hbTimer = null;
    }
    if (watchTimer) {
      try {
        clearInterval(watchTimer);
      } catch (e2) {}
      watchTimer = null;
    }
    onPageHide();
    try {
      if (bc) bc.close();
    } catch (e3) {}
    bc = null;
    ops("gpi_stop", { why: why || "stop", tab: tabId }, "info");
  }

  function snapshot() {
    return {
      started: started,
      tab: tabId,
      role: role,
      heavy: role === "heavy",
      can_heavy: canHeavyCollect(),
      lock_held: lockHeld,
      lease: leaseSnapshot(),
      progress: readJson(LS_PROGRESS),
      handoff: readHandoff(),
      peers: peerCount(),
      policy: policySnapshot(),
      stats: Object.assign({}, stats),
      cycle: sid(),
      server_has_b10: serverHasB10(),
    };
  }

  var api = {
    __ready: true,
    start: start,
    stop: stop,
    tryAcquireHeavy: tryAcquireHeavy,
    releaseHeavy: releaseHeavy,
    canHeavyCollect: canHeavyCollect,
    isHeavy: isHeavy,
    getRole: function () {
      return role;
    },
    getTabId: function () {
      return tabId;
    },
    peerCount: peerCount,
    policy: policySnapshot,
    snapshot: snapshot,
    writeProgress: writeProgress,
    serverHasB10: serverHasB10,
    processHandoff: processHandoff,
    clearHandoff: clearHandoff,
    readHandoff: readHandoff,
  };

  global.GROriginCoordinator = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- probe_method_matrix.js ---- */
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

/* ---- probe_self_heal.js ---- */
/**
 * greenpng Probe Self-Heal — Gap Table + Supervisor + loops T/C/B + adaptive.
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

/* ---- gr.seal.js ---- */
/**
 * GRSeal — session seal v2 (strict) + legacy v1 lab path.
 *
 * v2:
 *  - epoch/suite-bound grant key (server-derived)
 *  - envelope metadata: fe_epoch, suite_id, wasm_module_id, challenge_bind, pack_set_hash
 *  - AEAD via WebCrypto AES-256-GCM; KDF labels v2
 *  - bind digests MUST come from WASM module (wm-v2-s2-*) unless lab break-glass
 *
 * Master SEAL_SECRET never embedded.
 */
(function (global) {
  "use strict";

  var grant = null;
  var requireSealed = false;
  var wasmApi = null; // { module_id, suite_id, challenge_bind, pack_set_hash, attest }
  var wasmLoadP = null;
  var ALGO_DEFLATE = "aes256gcm+hmacsha256+deflate";
  var ALGO_IDENTITY = "aes256gcm+hmacsha256+identity";
  var SUITE_S2 = "s2-aesgcm-hkdf-v1";
  var WASM_ID = "wm-v2-s2-20260806";
  // Patched by scripts/build_fe_all.sh from fe/gr_seal_v2.wasm (0 = unset).
  var WASM_EXPECT_LEN = 28961;
  var WASM_EXPECT_SHA256 = "c5b97b9ad21c3dc3a47b02d55f1409dc1d4442409b14b2a4dd81cd0d45ef4409";

  function b64ToBytes(b64) {
    var bin = atob(b64);
    var out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function bytesToB64(u8) {
    var s = "";
    var chunk = 0x8000;
    for (var i = 0; i < u8.length; i += chunk) {
      s += String.fromCharCode.apply(null, u8.subarray(i, i + chunk));
    }
    return btoa(s);
  }

  function concatBytes(a, b) {
    var o = new Uint8Array(a.length + b.length);
    o.set(a, 0);
    o.set(b, a.length);
    return o;
  }

  function utf8(s) {
    return new TextEncoder().encode(s);
  }

  async function sha256(data) {
    var buf = await crypto.subtle.digest("SHA-256", data);
    return new Uint8Array(buf);
  }

  function toHex(u8) {
    var HEX = "0123456789abcdef";
    var s = "";
    for (var i = 0; i < u8.length; i++) {
      s += HEX[u8[i] >> 4] + HEX[u8[i] & 0xf];
    }
    return s;
  }

  /** v1 labels (lab only) */
  async function deriveKeysV1(secretBytes) {
    var enc = await sha256(concatBytes(utf8("gr-seal-enc-v1"), secretBytes));
    var mac = await sha256(concatBytes(utf8("gr-seal-mac-v1"), secretBytes));
    return { enc: enc, mac: mac };
  }

  /** v2 labels */
  async function deriveKeysV2(secretBytes) {
    var enc = await sha256(concatBytes(utf8("gr-seal-enc-v2"), secretBytes));
    var mac = await sha256(concatBytes(utf8("gr-seal-mac-v2"), secretBytes));
    return { enc: enc, mac: mac };
  }

  async function hmacSha256B64(macKeyBytes, parts) {
    var key = await crypto.subtle.importKey(
      "raw",
      macKeyBytes,
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign"]
    );
    var totalLen = 0;
    for (var i = 0; i < parts.length; i++) totalLen += parts[i].length;
    var msg = new Uint8Array(totalLen);
    var off = 0;
    for (var j = 0; j < parts.length; j++) {
      msg.set(parts[j], off);
      off += parts[j].length;
    }
    var sig = await crypto.subtle.sign("HMAC", key, msg);
    return bytesToB64(new Uint8Array(sig));
  }

  async function preparePlainForAes(u8) {
    if (typeof CompressionStream !== "undefined") {
      try {
        var compressed;
        if (typeof Blob !== "undefined" && typeof Response !== "undefined") {
          var stream = new Blob([u8]).stream().pipeThrough(new CompressionStream("deflate-raw"));
          compressed = new Uint8Array(await new Response(stream).arrayBuffer());
        } else {
          var cs = new CompressionStream("deflate-raw");
          var reader = cs.readable.getReader();
          var writer = cs.writable.getWriter();
          var readP = (async function () {
            var chunks = [];
            var total = 0;
            for (;;) {
              var n = await reader.read();
              if (n.done) break;
              chunks.push(n.value);
              total += n.value.length;
            }
            var out = new Uint8Array(total);
            var o = 0;
            for (var i = 0; i < chunks.length; i++) {
              out.set(chunks[i], o);
              o += chunks[i].length;
            }
            return out;
          })();
          await writer.write(u8);
          await writer.close();
          compressed = await readP;
        }
        if (compressed && compressed.length) {
          return { bytes: compressed, alg: ALGO_DEFLATE, deflated: true };
        }
      } catch (eDef) {}
    }
    return { bytes: u8, alg: ALGO_IDENTITY, deflated: false };
  }

  function grantIsV2(g) {
    if (!g) return false;
    return Number(g.seal_protocol || 0) >= 2 || g.algo === "gr_session_seal_v2" || !!g.suite_id;
  }

  function setGrant(g) {
    if (!g || typeof g !== "object") return;
    grant = {
      key_b64: g.key_b64 || g.keyB64 || "",
      exp_ms: Number(g.exp_ms || g.expMs || 0),
      session_id: g.session_id || g.sessionId || "",
      key_mode: g.key_mode || "session",
      seal_protocol: Number(g.seal_protocol || (g.algo === "gr_session_seal_v2" ? 2 : 1)),
      fe_epoch: g.fe_epoch || g.product_version || "",
      suite_id: g.suite_id || SUITE_S2,
      wasm_module_id: g.wasm_module_id || WASM_ID,
      wasm_url: g.wasm_url || g.wasm_url_flat || "",
      require_wasm: g.require_wasm !== false,
      challenge_seed: g.challenge_seed || "",
      algo: g.algo || "",
    };
    try {
      global.__GR_SEAL_GRANT__ = grant;
      global.__GR_BOOT__ = global.__GR_BOOT__ || {};
      global.__GR_BOOT__.seal_grant = grant;
      if (grant.fe_epoch) {
        global.__GR_FE_EPOCH__ = grant.fe_epoch;
      }
    } catch (e) {}
    // Kick WASM load as soon as grant arrives
    if (grantIsV2(grant)) {
      ensureWasm(grant).catch(function () {});
    }
  }

  function setRequireSealed(on) {
    requireSealed = !!on;
    try {
      global.__GR_REQUIRE_SEALED__ = requireSealed;
      global.__GR_BOOT__ = global.__GR_BOOT__ || {};
      if (requireSealed) global.__GR_BOOT__.require_sealed_ingest = true;
    } catch (e) {}
  }

  function adoptFromGlobals() {
    try {
      if (global.__GR_REQUIRE_SEALED__) requireSealed = true;
      var b = global.__GR_BOOT__ || {};
      if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) {
        requireSealed = true;
      }
      var g = global.__GR_SEAL_GRANT__ || b.seal_grant || null;
      if (g && (g.key_b64 || g.keyB64)) {
        if (!grant || !grant.key_b64) {
          setGrant(g);
        } else {
          var gExp = Number(g.exp_ms || g.expMs || 0);
          var gSid = g.session_id || g.sessionId || "";
          if (gSid && grant.session_id && gSid !== grant.session_id) {
            setGrant(g);
          } else if (gExp && (!grant.exp_ms || gExp > grant.exp_ms)) {
            setGrant(g);
          } else if (!grant.key_b64 && (g.key_b64 || g.keyB64)) {
            setGrant(g);
          }
        }
      }
      // bootstrap seal_v2 meta
      var meta = b.seal_v2 || (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) || null;
      if (meta && meta.wasm_url && grant && !grant.wasm_url) {
        grant.wasm_url = meta.wasm_url;
      }
    } catch (eAd) {}
  }

  function isRequireSealed() {
    adoptFromGlobals();
    if (requireSealed) return true;
    try {
      if (global.__GR_FORCE_PLAIN_INGEST__) return false;
      if (global.__GR_REQUIRE_SEALED__) return true;
      var b = global.__GR_BOOT__ || {};
      if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) return true;
      if (global.__GR_SEEN_SEALED_REQUIRED__) return true;
      if (global.__GR_FIRST_PARTY__ || b.first_party || b.firstParty) {
        if (b.env_id && String(b.env_id).indexOf("prod") === 0) return true;
        if (b.injectPath === "nginx" || b.inject_path === "nginx") return true;
      }
    } catch (e) {}
    return false;
  }

  function grantRawValid() {
    adoptFromGlobals();
    if (!grant || !grant.key_b64) return false;
    if (grant.exp_ms && Date.now() > grant.exp_ms) return false;
    return true;
  }

  function grantValid(sessionId) {
    if (!grantRawValid()) return false;
    if (sessionId && grant.session_id && grant.session_id !== sessionId) return false;
    return true;
  }

  function resolveSealSessionId(bodySid) {
    adoptFromGlobals();
    var want = String(bodySid || "");
    if (grantValid(want)) return want;
    var candidates = [];
    try {
      if (global.__GR_SESSION_ID__) candidates.push(String(global.__GR_SESSION_ID__));
      if (global.__GR_CYCLE_ID__) candidates.push(String(global.__GR_CYCLE_ID__));
      if (grant && grant.session_id) candidates.push(String(grant.session_id));
    } catch (eC) {}
    for (var i = 0; i < candidates.length; i++) {
      if (candidates[i] && grantValid(candidates[i])) return candidates[i];
    }
    if (grantRawValid() && grant.session_id) return String(grant.session_id);
    return want;
  }

  function waitForGrant(sessionId, timeoutMs) {
    var need = false;
    try {
      need = isRequireSealed();
    } catch (eN) {}
    timeoutMs = timeoutMs == null ? (need ? 16000 : 4000) : timeoutMs;
    adoptFromGlobals();
    if (grantValid(sessionId) || (sessionId && resolveSealSessionId(sessionId) && grantValid(resolveSealSessionId(sessionId)))) {
      return Promise.resolve(true);
    }
    if (grantRawValid()) return Promise.resolve(true);
    if (!need && !sessionId) return Promise.resolve(false);
    var start = Date.now();
    return new Promise(function (resolve) {
      function tick() {
        adoptFromGlobals();
        var sid = resolveSealSessionId(sessionId);
        if (grantValid(sid) || grantRawValid()) return resolve(true);
        if (Date.now() - start > timeoutMs) return resolve(false);
        setTimeout(tick, 35);
      }
      tick();
    });
  }

  /**
   * Resolve seal static URL for fe_load mode:
   * - first_party → same-origin /g5/dist/... (www)
   * - pv → https://pv.../dist/... (never /g5 on pv; never gv host)
   */
  function resolveSealAssetUrl(pathOrUrl) {
    var u = String(pathOrUrl || "");
    var b0 = global.__GR_BOOT__ || {};
    var assetRoot = String(
      b0.assetBase ||
        b0.script_base ||
        b0.first_party_path ||
        global.__GR_ASSET_BASE__ ||
        "/g5"
    ).replace(/\/$/, "");
    if (assetRoot.indexOf("/dist/v/") > 0 || /\/dist$/.test(assetRoot)) {
      assetRoot = assetRoot.replace(/\/dist\/.*$/, "").replace(/\/dist$/, "") || "/g5";
    }
    var fe = String(b0.fe_load || b0.feLoad || (global.__GR_FIRST_PARTY__ ? "first_party" : "") || "").toLowerCase();
    var pvMode = fe === "pv" || /^https?:\/\/pv\./i.test(assetRoot);
    // Never treat apiBase/gv as asset root
    try {
      var api = String(b0.apiBase || "").replace(/\/$/, "");
      if (api && assetRoot === api && /^https?:\/\/gv\./i.test(api)) {
        assetRoot = pvMode ? assetRoot : "/g5";
      }
    } catch (eApi) {}

    function underRoot(path) {
      var p = String(path || "");
      if (!p) return "";
      if (pvMode || /^https?:\/\//i.test(assetRoot)) {
        if (p.indexOf("/g5/") === 0 || p === "/g5") p = p.replace(/^\/g5(?=\/|$)/, "") || "/";
        try {
          return new URL(p, assetRoot + "/").href;
        } catch (eN) {
          return assetRoot.replace(/\/$/, "") + (p.charAt(0) === "/" ? p : "/" + p);
        }
      }
      // first_party: force document-origin absolute so dynamic import never follows gv
      if (p.charAt(0) === "/") {
        try {
          if (typeof location !== "undefined" && location.origin) {
            return location.origin + p;
          }
        } catch (eL) {}
        return p;
      }
      return (assetRoot || "/g5") + "/" + p.replace(/^\//, "");
    }

    if (!u) return underRoot("/g5/dist/gr_seal_v2.wasm");
    if (/^https?:\/\//i.test(u)) {
      try {
        var abs = new URL(u);
        if (/^gv\./i.test(abs.hostname)) {
          // remap onto asset root
          var pathG = abs.pathname || "/";
          if (pathG.indexOf("/g5/") === 0) pathG = pathG.replace(/^\/g5(?=\/|$)/, "") || "/";
          return underRoot(pathG);
        }
        return abs.href;
      } catch (eA) {
        return u;
      }
    }
    return underRoot(u.charAt(0) === "/" ? u : "/" + u);
  }

  function wasmEnvSupported() {
    try {
      return (
        typeof WebAssembly !== "undefined" &&
        typeof WebAssembly.instantiate === "function" &&
        typeof WebAssembly.Module === "function"
      );
    } catch (eW) {
      return false;
    }
  }

  function looksLikeWasmBinary(u8) {
    // \0asm magic
    return (
      u8 &&
      u8.length >= 8 &&
      u8[0] === 0x00 &&
      u8[1] === 0x61 &&
      u8[2] === 0x73 &&
      u8[3] === 0x6d
    );
  }

  function wasmExpectFromBoot(g) {
    var meta =
      (g && g.seal_v2) ||
      (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2) ||
      (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) ||
      {};
    var len =
      Number(meta.wasm_bytes || meta.wasm_len || g.wasm_bytes || WASM_EXPECT_LEN || 0) || 0;
    var sha = String(
      meta.wasm_sha256 || meta.wasm_sha || g.wasm_sha256 || WASM_EXPECT_SHA256 || ""
    )
      .trim()
      .toLowerCase();
    return { len: len, sha256: sha };
  }

  async function sha256Hex(buf) {
    var dig = await crypto.subtle.digest("SHA-256", buf);
    var u8 = new Uint8Array(dig);
    var hex = "";
    for (var i = 0; i < u8.length; i++) {
      hex += (u8[i] + 256).toString(16).slice(1);
    }
    return hex;
  }

  /**
   * Fetch wasm bytes with integrity checks. Always arrayBuffer-first
   * (small seal module; MIME/truncation/HTML-intercept is common on bots/CF).
   */
  async function fetchWasmBytes(wasmUrl, g) {
    var resp = await fetch(wasmUrl, {
      cache: "no-cache",
      credentials: "same-origin",
      mode: "cors",
    });
    if (!resp.ok) throw new Error("wasm_http_" + resp.status);
    var ct = "";
    try {
      ct = String(resp.headers.get("Content-Type") || "").toLowerCase();
    } catch (eC) {}
    var buf = await resp.arrayBuffer();
    var u8 = new Uint8Array(buf);
    if (!looksLikeWasmBinary(u8)) {
      // HTML/JSON intercept (login wall / CF / 404 page) → not a wasm module
      var snip = "";
      try {
        snip = new TextDecoder().decode(u8.subarray(0, 48)).replace(/\s+/g, " ");
      } catch (eD) {}
      throw new Error(
        "wasm_not_binary:ct=" +
          (ct || "?").slice(0, 40) +
          ";len=" +
          u8.length +
          ";snip=" +
          snip.slice(0, 40)
      );
    }
    var expect = wasmExpectFromBoot(g || {});
    if (expect.len > 0 && u8.length !== expect.len) {
      throw new Error(
        "wasm_len_mismatch:got=" + u8.length + ";want=" + expect.len
      );
    }
    if (expect.sha256 && expect.sha256.length >= 32) {
      var got = await sha256Hex(buf);
      if (got !== expect.sha256) {
        throw new Error(
          "wasm_sha256_mismatch:got=" + got.slice(0, 16) + ";want=" + expect.sha256.slice(0, 16)
        );
      }
    }
    return buf;
  }

  /**
   * Load WASM seal helper (required for v2 under require_wasm).
   */
  function ensureWasm(g) {
    if (wasmApi) return Promise.resolve(wasmApi);
    if (wasmLoadP) return wasmLoadP;
    // Permanent fail short-circuit (crawlers without WASM / blocked wasm assets).
    if (global.__GR_SEAL_WASM_DEAD__) {
      return Promise.reject(
        new Error(String(global.__GR_SEAL_WASM_DEAD_ERR__ || "seal_wasm_required: dead"))
      );
    }
    g = g || grant || {};
    var url =
      g.wasm_url ||
      (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2 && global.__GR_BOOT__.seal_v2.wasm_url) ||
      (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2 && global.__GR_MANIFEST__.seal_v2.wasm_url) ||
      "";
    if (!url) {
      // Standard C: never use /dist/v/<version>/ — prefer bootstrap hashed URLs.
      try {
        var meta0 =
          (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2) ||
          null;
        if (meta0 && meta0.wasm_url) url = String(meta0.wasm_url);
      } catch (eM0) {}
      if (!url) {
        var gen0 = "";
        try {
          gen0 = String(
            (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_gen) ||
              global.__GR_ASSET_GEN__ ||
              ""
          );
        } catch (eG0) {}
        url = gen0
          ? "/g5/dist/gr_seal_v2." + gen0 + ".wasm"
          : "/g5/dist/gr_seal_v2.wasm";
      }
    }
    // Collapse sticky version paths from old grants.
    url = String(url).replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    url = resolveSealAssetUrl(url);
    // Prefer ES module glue when present (preserve content-hash token if any).
    var glue = "";
    try {
      var metaL =
        (g && g.loader_url) ||
        (global.__GR_MANIFEST__ &&
          global.__GR_MANIFEST__.seal_v2 &&
          global.__GR_MANIFEST__.seal_v2.loader_url) ||
        (global.__GR_BOOT__ &&
          global.__GR_BOOT__.seal_v2 &&
          global.__GR_BOOT__.seal_v2.loader_url) ||
        "";
      if (metaL) glue = String(metaL);
    } catch (eML) {}
    if (!glue) {
      // Map hashed wasm → hashed loader when possible: gr_seal_v2.<h>.wasm → loader.<h>.js
      var wm = String(url).match(/gr_seal_v2(?:\.([a-f0-9]{8,16}))?\.wasm/i);
      if (wm && wm[1]) {
        glue = String(url).replace(
          /gr_seal_v2(?:\.[a-f0-9]{8,16})?\.wasm.*/i,
          "gr_seal_v2_loader." + wm[1] + ".js"
        );
      } else {
        glue =
          (url.indexOf(".wasm") > 0
            ? url.replace(/gr_seal_v2(?:\.[a-f0-9]{8,16})?\.wasm.*/i, "gr_seal_v2_loader.js")
            : "") || url.replace(/\.wasm.*/, "_loader.js");
      }
    }
    glue = String(glue).replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    glue = resolveSealAssetUrl(glue);

    wasmLoadP = (async function () {
      if (!wasmEnvSupported()) {
        var eUn = new Error("seal_wasm_unsupported: WebAssembly API missing");
        global.__GR_SEAL_WASM_DEAD__ = 1;
        global.__GR_SEAL_WASM_DEAD_ERR__ = eUn.message;
        global.__GR_SEAL_WASM_CAP__ = { ok: false, reason: "no_webassembly_api" };
        throw eUn;
      }
      global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "api_present" };
      var lastErr = null;
      // Path A: wasm-bindgen ES module glue (preferred — full exports)
      try {
        var loaderUrl = glue;
        if (!loaderUrl) {
          loaderUrl = resolveSealAssetUrl("/g5/dist/gr_seal_v2_loader.js");
        }
        var mod = await import(/* webpackIgnore: true */ loaderUrl);
        // Bytes-first: validate magic/len/sha256, then initSync / default(bytes).
        // Avoid instantiateStreaming (MIME/truncation/CF HTML) for ~29KB seal.
        var bytesValidated = await fetchWasmBytes(url, g);
        if (mod && typeof mod.initSync === "function") {
          mod.initSync({ module: bytesValidated });
        } else if (mod && mod.default) {
          await mod.default(bytesValidated);
        } else {
          throw new Error("wasm_glue_no_init");
        }
        var api = {
          module_id: mod.module_id ? mod.module_id() : WASM_ID,
          suite_id: mod.suite_id ? mod.suite_id() : SUITE_S2,
          challenge_bind: function (sid, seed, epoch) {
            return mod.challenge_bind(sid, seed, epoch);
          },
          pack_set_hash: function (joined) {
            return mod.pack_set_hash(joined);
          },
        };
        wasmApi = api;
        global.__GR_SEAL_WASM__ = api;
        global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "glue_ok", url: String(url).slice(0, 120) };
        return api;
      } catch (eImp) {
        lastErr = eImp;
        // Path B: raw instantiate from validated bytes (no glue exports → still fail closed)
        try {
          var bytes = await fetchWasmBytes(url, g);
          var result = await WebAssembly.instantiate(bytes, {});
          var exp = result.instance && result.instance.exports;
          if (!exp || typeof exp.challenge_bind !== "function") {
            throw new Error("wasm_no_exports");
          }
          var apiRaw = {
            module_id: WASM_ID,
            suite_id: SUITE_S2,
            challenge_bind: function (sid, seed, epoch) {
              return exp.challenge_bind(sid, seed, epoch);
            },
            pack_set_hash: function (joined) {
              return exp.pack_set_hash(joined);
            },
          };
          wasmApi = apiRaw;
          global.__GR_SEAL_WASM__ = apiRaw;
          global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "raw_instantiate", url: String(url).slice(0, 120) };
          return apiRaw;
        } catch (eRaw) {
          lastErr = eRaw || eImp;
          // Path C: lab-only pure JS bind (identical digests) — only if allowed
          var allowJs =
            global.__GR_SEAL_ALLOW_JS_BIND__ ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.seal_allow_js_bind);
          if (!allowJs && isRequireSealed()) {
            var msg =
              "seal_wasm_required: " +
              String((lastErr && lastErr.message) || lastErr || eImp || "load_failed");
            global.__GR_SEAL_WASM_DEAD__ = 1;
            global.__GR_SEAL_WASM_DEAD_ERR__ = msg.slice(0, 200);
            global.__GR_SEAL_WASM_CAP__ = {
              ok: false,
              reason: "load_failed",
              err: msg.slice(0, 160),
              url: String(url).slice(0, 120),
            };
            // Clear load promise so... actually keep dead short-circuit via DEAD flag
            throw new Error(msg);
          }
          wasmApi = pureJsBindApi();
          global.__GR_SEAL_WASM__ = wasmApi;
          global.__GR_SEAL_WASM_JS_FALLBACK__ = 1;
          global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "js_fallback" };
          return wasmApi;
        }
      }
    })().catch(function (eFail) {
      // Allow one future retry only if not marked permanent dead.
      if (!global.__GR_SEAL_WASM_DEAD__) wasmLoadP = null;
      throw eFail;
    });

    return wasmLoadP;
  }

  /** Pure JS bind matching Rust/WASM digest formulas (lab fallback only). */
  function pureJsBindApi() {
    async function shaHex(parts) {
      var total = 0;
      for (var i = 0; i < parts.length; i++) total += parts[i].length;
      var msg = new Uint8Array(total);
      var o = 0;
      for (var j = 0; j < parts.length; j++) {
        msg.set(parts[j], o);
        o += parts[j].length;
      }
      return toHex(await sha256(msg));
    }
    return {
      module_id: WASM_ID,
      suite_id: SUITE_S2,
      challenge_bind: function (sid, seed, epoch) {
        // sync wrapper using cached promise — make async at call site
        return null;
      },
      challenge_bind_async: async function (sid, seed, epoch) {
        return shaHex([
          utf8("gr-challenge-bind-v2|"),
          utf8(String(sid || "")),
          utf8("|"),
          utf8(String(seed || "")),
          utf8("|"),
          utf8(String(epoch || "")),
        ]);
      },
      pack_set_hash: function () {
        return null;
      },
      pack_set_hash_async: async function (joined) {
        return shaHex([utf8("gr-pack-set-v2|"), utf8(String(joined || ""))]);
      },
      async_mode: true,
    };
  }

  async function resolveBind(api, kind, a, b, c) {
    if (!api) throw new Error("seal_wasm_missing");
    if (kind === "challenge") {
      if (api.async_mode && api.challenge_bind_async) return api.challenge_bind_async(a, b, c);
      if (typeof api.challenge_bind === "function") {
        var r = api.challenge_bind(a, b, c);
        if (r && typeof r.then === "function") return r;
        if (r) return r;
      }
      // JS fallback digest
      return (await pureJsBindApi().challenge_bind_async(a, b, c));
    }
    if (kind === "pack") {
      if (api.async_mode && api.pack_set_hash_async) return api.pack_set_hash_async(a);
      if (typeof api.pack_set_hash === "function") {
        var r2 = api.pack_set_hash(a);
        if (r2 && typeof r2.then === "function") return r2;
        if (r2) return r2;
      }
      return pureJsBindApi().pack_set_hash_async(a);
    }
    throw new Error("bind_kind");
  }

  function buildPackSetParts(body) {
    var epoch =
      (grant && grant.fe_epoch) ||
      global.__GR_SERVER_PRODUCT_VERSION__ ||
      global.__GR_PRODUCT_VERSION__ ||
      "";
    var bid = String((body && body.batch_id) || "");
    var fields =
      (body && body.payload && body.payload.fields) ||
      (body && body.fields) ||
      {};
    var algo = String(fields.cpu_loop_algo || fields.cpu_loop_algo_build || "");
    var feImpl = String(fields.fe_impl_version || fields.fe_packs_version || "");
    var hard = "";
    try {
      hard = String(global.__GR_FE_HARD_IMPL__ || (global.__GR_FE_IMPL__ && global.__GR_FE_IMPL__.hard) || "");
    } catch (eH) {}
    // probe_dag_v2 decision (design §9.3): version + engine claim enter the
    // pack-set binding so the server can explain "why was field X not executed"
    // from the sealed envelope alone, without trusting any later batch.
    var dagVer = "";
    var dagEngine = "";
    try {
      var consumed = global.__GR_DAG_V2_CONSUMED__ || null;
      if (consumed) {
        dagVer = String(consumed.version || 0);
        dagEngine = String(consumed.engine || "unknown");
      }
    } catch (eDag) {}
    // Join with | matching server compute_pack_set_hash parts
    return [epoch, bid, algo || "-", feImpl || hard || "-", WASM_ID, dagVer || "0", dagEngine || "unknown"].join("|");
  }

  function challengeSeedMaterial() {
    try {
      if (grant && grant.challenge_seed) return String(grant.challenge_seed);
      var b = global.__GR_BOOT__ || {};
      if (b.challenge_seed) return String(b.challenge_seed);
      if (global.__GR_CHALLENGE_SEED__) return String(global.__GR_CHALLENGE_SEED__);
      // open may store on cycle
      if (global.__GR_OPEN__ && global.__GR_OPEN__.challenge_seed) {
        return String(global.__GR_OPEN__.challenge_seed);
      }
    } catch (e) {}
    // Deterministic non-empty bind even without seed (still epoch+session bound in WASM formula)
    return "open_seed_pending";
  }

  async function sealIngestBodyV2(body) {
    if (!global.crypto || !crypto.subtle) throw new Error("webcrypto_unavailable");
    adoptFromGlobals();
    var sid = resolveSealSessionId(String((body && body.session_id) || ""));
    if (body && sid && body.session_id !== sid) body.session_id = sid;
    if (!grantValid(sid)) throw new Error("seal_grant_missing");
    if (!grantIsV2(grant)) throw new Error("seal_grant_not_v2");

    var api = await ensureWasm(grant);
    var epoch = grant.fe_epoch || global.__GR_PRODUCT_VERSION__ || "";
    var suite = grant.suite_id || SUITE_S2;
    var wasmId = (api && api.module_id) || grant.wasm_module_id || WASM_ID;
    var seed = challengeSeedMaterial();
    var chBind = await resolveBind(api, "challenge", sid, seed, epoch);
    var packParts = buildPackSetParts(body);
    var psh = await resolveBind(api, "pack", packParts);

    // Stamp client honesty fields on payload when missing
    try {
      body.payload = body.payload || {};
      body.payload.fields = body.payload.fields || {};
      var f = body.payload.fields;
      if (!f.fe_impl_version) {
        f.fe_impl_version =
          global.__GR_FE_IMPL_VERSION__ ||
          (global.__GR_FE_IMPL__ && global.__GR_FE_IMPL__.content) ||
          epoch;
      }
      if (!f.fe_packs_version) f.fe_packs_version = f.fe_impl_version;
      f.client_fe_epoch = epoch;
      f.seal_suite_id = suite;
      f.seal_wasm_module_id = wasmId;
      // DAG v2 decision stamp (best-effort; missing when no plan was applied yet).
      var dagStamp = (global.__GR_DAG_V2_CONSUMED__) || null;
      if (dagStamp) {
        f.dag_v2_version = dagStamp.version || 0;
        f.dag_v2_engine = dagStamp.engine || "unknown";
      }
    } catch (eSt) {}

    var secret = b64ToBytes(grant.key_b64);
    var keys = await deriveKeysV2(secret);
    var plain = utf8(JSON.stringify(body));
    var pre = await preparePlainForAes(plain);
    var nonce = new Uint8Array(12);
    crypto.getRandomValues(nonce);
    var aesKey = await crypto.subtle.importKey("raw", keys.enc, { name: "AES-GCM" }, false, [
      "encrypt",
    ]);
    var ctBuf = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      aesKey,
      pre.bytes
    );
    var ct = new Uint8Array(ctBuf);
    var vt =
      (body.payload && body.payload.fields && body.payload.fields.visitor_terminal_id) ||
      (body.payload && body.payload.visitor_terminal_id) ||
      global.__GR_VTID__ ||
      "";
    try {
      if (!vt && global.GRStorage && GRStorage.visitorTerminalId) {
        vt = GRStorage.visitorTerminalId() || "";
      }
    } catch (e) {}
    var env = {
      v: 2,
      alg: pre.alg,
      session_id: sid,
      visitor_terminal_id: String(vt || "vt_unknown"),
      batch_id: String((body && body.batch_id) || ""),
      nonce_b64: bytesToB64(nonce),
      ciphertext_b64: bytesToB64(ct),
      sig_b64: "",
      key_mode: "session",
      seal_exp_ms: grant.exp_ms || 0,
      suite_id: suite,
      fe_epoch: epoch,
      wasm_module_id: wasmId,
      challenge_bind: String(chBind || ""),
      pack_set_hash: String(psh || ""),
    };
    env.sig_b64 = await hmacSha256B64(keys.mac, [
      utf8("v2|"),
      utf8(env.session_id),
      utf8("|"),
      utf8(env.visitor_terminal_id),
      utf8("|"),
      utf8(env.batch_id),
      utf8("|"),
      utf8(env.suite_id),
      utf8("|"),
      utf8(env.fe_epoch),
      utf8("|"),
      utf8(env.wasm_module_id),
      utf8("|"),
      utf8(env.challenge_bind),
      utf8("|"),
      utf8(env.pack_set_hash),
      utf8("|"),
      utf8(env.nonce_b64),
      utf8("|"),
      utf8(env.ciphertext_b64),
    ]);
    return env;
  }

  /** Legacy v1 seal (lab only when grant is v1). */
  async function sealIngestBodyV1(body) {
    if (!global.crypto || !crypto.subtle) throw new Error("webcrypto_unavailable");
    adoptFromGlobals();
    var sid = resolveSealSessionId(String((body && body.session_id) || ""));
    if (body && sid && body.session_id !== sid) body.session_id = sid;
    if (!grantValid(sid)) throw new Error("seal_grant_missing");
    var secret = b64ToBytes(grant.key_b64);
    var keys = await deriveKeysV1(secret);
    var plain = utf8(JSON.stringify(body));
    var pre = await preparePlainForAes(plain);
    var nonce = new Uint8Array(12);
    crypto.getRandomValues(nonce);
    var aesKey = await crypto.subtle.importKey("raw", keys.enc, { name: "AES-GCM" }, false, [
      "encrypt",
    ]);
    var ctBuf = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      aesKey,
      pre.bytes
    );
    var ct = new Uint8Array(ctBuf);
    var vt =
      (body.payload && body.payload.fields && body.payload.fields.visitor_terminal_id) ||
      global.__GR_VTID__ ||
      "vt_unknown";
    var env = {
      v: 1,
      alg: pre.alg,
      session_id: sid,
      visitor_terminal_id: String(vt || "vt_unknown"),
      batch_id: String((body && body.batch_id) || ""),
      nonce_b64: bytesToB64(nonce),
      ciphertext_b64: bytesToB64(ct),
      sig_b64: "",
      key_mode: "session",
      seal_exp_ms: grant.exp_ms || 0,
    };
    env.sig_b64 = await hmacSha256B64(keys.mac, [
      utf8("v1|"),
      utf8(env.session_id),
      utf8("|"),
      utf8(env.visitor_terminal_id),
      utf8("|"),
      utf8(env.batch_id),
      utf8("|"),
      utf8(env.nonce_b64),
      utf8("|"),
      utf8(env.ciphertext_b64),
    ]);
    return env;
  }

  async function sealIngestBody(body) {
    adoptFromGlobals();
    if (grantIsV2(grant)) {
      return sealIngestBodyV2(body);
    }
    // Production require sealed should not hit v1 — open always issues v2 when require_v2
    if (isRequireSealed() && !(global.__GR_SEAL_ALLOW_V1__ || global.__GR_FORCE_PLAIN_INGEST__)) {
      throw new Error("seal_v2_required");
    }
    return sealIngestBodyV1(body);
  }

  async function prepareUpload(apiBase, body) {
    var base = String(apiBase || "").replace(/\/$/, "");
    adoptFromGlobals();
    var need = isRequireSealed();
    var forcePlain = !!global.__GR_FORCE_PLAIN_INGEST__;
    var sid = (body && body.session_id) || "";
    if (forcePlain && !need) {
      return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
    }
    if (!need) {
      await waitForGrant(sid, 400);
      sid = resolveSealSessionId(sid);
      if (body && sid) body.session_id = sid;
      if (!grantValid(sid)) {
        return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
      }
    } else {
      var bid0 = String((body && body.batch_id) || "");
      var heavySeal =
        bid0.indexOf("B10") === 0 ||
        bid0.indexOf("B10x_") === 0 ||
        bid0 === "B0_bootstrap" ||
        bid0 === "B7_sandbox";
      await waitForGrant(sid, heavySeal ? 20000 : 16000);
      sid = resolveSealSessionId(sid);
      if (body && sid) body.session_id = sid;
      if (!grantValid(sid) && !grantRawValid()) throw new Error("seal_grant_unavailable");
      if (!grantValid(sid) && grantRawValid() && grant.session_id) {
        sid = String(grant.session_id);
        if (body) body.session_id = sid;
      }
      if (!grantValid(sid)) throw new Error("seal_grant_unavailable");
      // Preload WASM during wait window
      if (grantIsV2(grant)) {
        try {
          await ensureWasm(grant);
        } catch (eW) {
          throw eW;
        }
      }
    }
    try {
      var bid1 = String((body && body.batch_id) || "");
      var heavyCrypto =
        bid1.indexOf("B10x_") === 0 || bid1 === "B10_hw_curves" || bid1 === "B7_sandbox";
      var sealTimeoutMs = need ? (heavyCrypto ? 18000 : 12000) : 2500;
      var env = await Promise.race([
        sealIngestBody(body),
        new Promise(function (_, reject) {
          setTimeout(function () {
            reject(new Error("seal_timeout"));
          }, sealTimeoutMs);
        }),
      ]);
      return {
        url: base + "/v1/ingest/sealed",
        body: JSON.stringify(env),
        sealed: true,
        session_id: sid,
        seal_v: env.v,
      };
    } catch (e) {
      if (need || isRequireSealed()) throw e;
      return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
    }
  }

  function preferKeepaliveOverBeacon() {
    return true;
  }

  adoptFromGlobals();

  global.GRSeal = {
    setGrant: setGrant,
    setRequireSealed: setRequireSealed,
    isRequireSealed: isRequireSealed,
    grantValid: grantValid,
    grantRawValid: grantRawValid,
    adoptFromGlobals: adoptFromGlobals,
    resolveSealSessionId: resolveSealSessionId,
    waitForGrant: waitForGrant,
    sealIngestBody: sealIngestBody,
    prepareUpload: prepareUpload,
    preferKeepaliveOverBeacon: preferKeepaliveOverBeacon,
    ensureWasm: ensureWasm,
    hasCompressionStream: function () {
      return typeof CompressionStream !== "undefined";
    },
    version: "gr_session_seal_v2",
    suite: SUITE_S2,
    wasmModuleId: WASM_ID,
  };
})(typeof window !== "undefined" ? window : globalThis);

/* ---- upload_queue.js ---- */
/**
 * GR UploadQueue — sub-pack self-managed upload (design from v57 upload_queue).
 * Priority sorts kick/upload order only; items never wait on each other to finish.
 */
(function (global) {
  "use strict";
  var cfg = {
    // iss/70 P2: start conservative; AIMD raises up to concurrency_max.
    concurrency: 3,
    concurrency_max: 6,
    concurrency_floor: 1,
    ramp_after_first: 4,
    /** Soft mid-ramp when B0 is only enqueued (not yet ok). */
    mid_ramp_concurrency: 4,
    apiBase: "",
    session_id: "",
    inject_path: "app",
    alive_retry_ms: 30000,
    /** Soft/mid default cap; hard anchors use lifecycle hard_max (see maxAttemptsForItem). */
    max_attempts_alive: 5,
    max_attempts_hide: 5,
    /** Hard commercial anchors (B10 / form-carrying lite) get extra priority on pagehide. */
    hard_anchor_batches: [
      "B10_hw_curves",
      "B0_bootstrap",
      "B2_hardware",
      "B3_system",
      "B1_conflict",
      "B12_anti_camouflage",
      "B8_gateway",
      "B8_gateway_early",
      // Silicon deepen: treat as pagehide-hard when residual completeness matters
      "B10x_silicon_ulp",
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
    ],
    hard_anchor_priority_boost: 1000,
    /** Extra boost after B10 lands so B10x outruns mid packs on short dwell. */
    b10x_post_b10_boost: 1500,
  };
  var pending = [];
  var inflight = 0;
  /** Items currently in postOne (for material-state / hard-SLA checks). */
  var inflightItems = [];
  /** Concurrent heavy (B10x/B7) uploads — cap 1 to reduce Failed to fetch. */
  var heavyInflight = 0;
  /** Counters for iss/70 P0/P1 observability (collect vs transport separation). */
  var transportRetryCount = 0;
  var captureFreezeCount = 0;
  var ackStored = 0;
  var ackDuplicate = 0;
  var ackMerged = 0;
  var ackConflict = 0;
  var aimdDowns = 0;
  var aimdUps = 0;
  /** Material Registry: batch_id → record (generation, hash, state, ack). */
  var materialRegistry = Object.create(null);
  /** Rolling success window for AIMD. */
  var recentUploadOk = 0;
  var recentUploadFail = 0;
  var sent = 0;
  var failed = 0;
  var skipped = 0;
  var hardFlushed = 0;
  var hardRetries = 0;
  var attempts = Object.create(null);
  var sentKeys = Object.create(null); // GA4-like: do not re-upload already-sent batch keys
  var flushReason = null;
  var hiding = false;
  var ramped = false;
  /** When set, identity/session uploads stop (cool / cycle_complete / 410). */
  var haltState = null; // { reason, session_id, code, at_ms }
  var haltedDrops = 0;
  /** In-flight AbortControllers — aborted on halt so browser does not complete more 410s. */
  var inflightCtrls = [];
  /** Session-level upload outcomes (P2 observability). */
  var sessionOutcome = {
    sealed_ok: 0,
    sealed_reject: 0,
    plain_ok: 0,
    network: 0,
    other_fail: 0,
    batches_ok: Object.create(null),
    /** Heartbeat/start markers only — must not block final payload. */
    batches_started: Object.create(null),
  };
  var outcomeReported = false;
  /** Counts soft/mid drops on pagehide and success short-circuits (ops). */
  var pagehideDropped = 0;
  var successShortCircuit = 0;
  var softAbortOnUnload = 0;
  /** force_recollect budget: max 1 per batch after terminal ok (iss/65). */
  var forceRecollectUsed = Object.create(null);

  /** Must-land silicon trio (EDH); deep is best-effort after these. */
  var B10X_MUST = ["B10x_silicon_ulp", "B10x_silicon_noderiv", "B10x_silicon_rint"];

  function batchAlreadyOk(batchId) {
    try {
      return !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok[String(batchId || "")]);
    } catch (e) {
      return false;
    }
  }

  /** Stable JSON for material hash (sorted object keys). */
  function stableStringify(v) {
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    if (Array.isArray(v)) {
      return "[" + v.map(stableStringify).join(",") + "]";
    }
    var keys = Object.keys(v).sort();
    var parts = [];
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i];
      parts.push(JSON.stringify(k) + ":" + stableStringify(v[k]));
    }
    return "{" + parts.join(",") + "}";
  }

  /** Sync SHA-256 hex (browser + node). Material identity only. */
  function sha256Hex(str) {
    try {
      if (typeof require === "function") {
        var c = require("crypto");
        if (c && c.createHash) return c.createHash("sha256").update(String(str), "utf8").digest("hex");
      }
    } catch (eN) {}
    // Minimal pure-js SHA-256 for browser lab (not crypto-grade multi-block edge; fine for dedupe).
    function rotr(n, x) {
      return (x >>> n) | (x << (32 - n));
    }
    var K = [
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
      0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
      0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
      0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
      0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
      0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
      0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
      0xc67178f2,
    ];
    function toBytes(s) {
      var utf8 = unescape(encodeURIComponent(String(s)));
      var arr = [];
      for (var i = 0; i < utf8.length; i++) arr.push(utf8.charCodeAt(i) & 255);
      return arr;
    }
    var bytes = toBytes(str);
    var l = bytes.length;
    var bitLen = l * 8;
    bytes.push(0x80);
    while ((bytes.length % 64) !== 56) bytes.push(0);
    for (var i = 7; i >= 0; i--) bytes.push((bitLen / Math.pow(2, i * 8)) & 255);
    var H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for (var off = 0; off < bytes.length; off += 64) {
      var w = new Array(64);
      for (var t = 0; t < 16; t++) {
        var j = off + t * 4;
        w[t] = (bytes[j] << 24) | (bytes[j + 1] << 16) | (bytes[j + 2] << 8) | bytes[j + 3];
      }
      for (t = 16; t < 64; t++) {
        var s0 = rotr(7, w[t - 15]) ^ rotr(18, w[t - 15]) ^ (w[t - 15] >>> 3);
        var s1 = rotr(17, w[t - 2]) ^ rotr(19, w[t - 2]) ^ (w[t - 2] >>> 10);
        w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0;
      }
      var a = H[0],
        b = H[1],
        c2 = H[2],
        d = H[3],
        e = H[4],
        f = H[5],
        g = H[6],
        h = H[7];
      for (t = 0; t < 64; t++) {
        var S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
        var ch = (e & f) ^ (~e & g);
        var t1 = (h + S1 + ch + K[t] + w[t]) | 0;
        var S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
        var maj = (a & b) ^ (a & c2) ^ (b & c2);
        var t2 = (S0 + maj) | 0;
        h = g;
        g = f;
        f = e;
        e = (d + t1) | 0;
        d = c2;
        c2 = b;
        b = a;
        a = (t1 + t2) | 0;
      }
      H[0] = (H[0] + a) | 0;
      H[1] = (H[1] + b) | 0;
      H[2] = (H[2] + c2) | 0;
      H[3] = (H[3] + d) | 0;
      H[4] = (H[4] + e) | 0;
      H[5] = (H[5] + f) | 0;
      H[6] = (H[6] + g) | 0;
      H[7] = (H[7] + h) | 0;
    }
    var out = "";
    for (i = 0; i < 8; i++) {
      var hx = (H[i] >>> 0).toString(16);
      out += ("00000000" + hx).slice(-8);
    }
    return out;
  }

  /**
   * iss/72: material hash = SHA-256 of canonical inner fields only.
   * Request-body SHA / attempt_id / seal envelope are NOT material identity.
   */
  function payloadHashOf(payload) {
    try {
      var fields = payload && payload.fields != null ? payload.fields : payload || {};
      var s = stableStringify(fields);
      return "sha256:" + sha256Hex(s);
    } catch (e) {
      return "sha256:0";
    }
  }

  /** logical key: session|batch|source|generation (iss/72) */
  function registryKey(sessionId, batchId, source, generation) {
    return (
      String(sessionId || cfg.session_id || "") +
      "|" +
      String(batchId || "") +
      "|" +
      String(source || "main") +
      "|" +
      String(generation != null ? generation : 1)
    );
  }

  function registryGet(batchId, source, sessionId, generation) {
    var sid = sessionId != null ? sessionId : cfg.session_id || "";
    var gen = generation != null ? generation : 1;
    return materialRegistry[registryKey(sid, batchId, source, gen)] || null;
  }

  function registryPut(rec) {
    if (!rec || !rec.batch_id) return;
    var sid = rec.session_id || cfg.session_id || "";
    var gen = rec.material_generation != null ? rec.material_generation : 1;
    materialRegistry[registryKey(sid, rec.batch_id, rec.source || "main", gen)] = rec;
  }

  /**
   * iss/70 Material Registry snapshot for a logical batch.
   * States: new|collected|queued|uploading|retry_wait|acked|conflict|transport_ok_unverified|absent
   */
  function registryUpsertFromItem(item, state) {
    if (!item || !item.batch_id) return null;
    var src = item.source || "main";
    var sid = item.session_id || cfg.session_id || "";
    var gen = item.material_generation != null ? item.material_generation : 1;
    var prev = registryGet(item.batch_id, src, sid, gen) || {};
    var rec = {
      batch_id: String(item.batch_id),
      source: src,
      session_id: sid,
      capture_id: item.capture_id || prev.capture_id || null,
      material_generation: gen,
      material_hash: item.payload_hash || prev.material_hash || null,
      payload_hash: item.payload_hash || prev.payload_hash || null,
      request_hash: prev.request_hash || null,
      state: state || prev.state || "collected",
      ack_type: prev.ack_type || null,
      transport_attempts: item._transport_attempts || prev.transport_attempts || 0,
      updated_ms: Date.now(),
    };
    registryPut(rec);
    return rec;
  }

  /**
   * Apply server ACK (iss/72 strict).
   * Only explicit ack.type in {stored,duplicate,merged} → ACKED.
   * Missing/empty ack → transport_ok_unverified (NOT batches_ok).
   */
  function applyAck(item, j) {
    j = j || {};
    var ack = j.ack && typeof j.ack === "object" ? j.ack : null;
    var type = ack && ack.type ? String(ack.type) : "";
    var bid = String((item && item.batch_id) || "");
    var src = (item && item.source) || "main";
    var sid = (item && item.session_id) || cfg.session_id || "";
    var gen =
      (ack && ack.material_generation != null
        ? ack.material_generation
        : item && item.material_generation != null
          ? item.material_generation
          : 1) | 0;
    // Prefer client material hash for registry identity; server body hash is request_hash.
    var materialHash =
      (ack && (ack.material_hash || ack.client_payload_hash)) ||
      (item && item.payload_hash) ||
      null;
    var requestHash = (ack && ack.payload_hash) || j.payload_hash || null;
    if (!type) {
      if (j.conflict === true || j.ack_conflict === true) type = "conflict";
      else if (j.duplicate === true || j.cold_skipped_unchanged === true) type = "duplicate";
      else if (j.merged === true) type = "merged";
      else if (j.ok === false && j.error) type = "rejected";
      else if (j.empty_body === true) type = "transport_ok_unverified";
      else if (j.ok === true && !ack) type = "transport_ok_unverified";
      else type = "transport_ok_unverified";
    }
    // Validate key match when server provides fields.
    // Explicit server type stored|duplicate|merged is authoritative (iss/72).
    // Do not reclassify server-success into conflict on minor echo mismatches
    // (sealed path may omit/reshape capture_id; material_hash remains client-side).
    var serverExplicitOk =
      type === "stored" || type === "duplicate" || type === "merged";
    if (ack && !serverExplicitOk) {
      if (ack.batch_id && String(ack.batch_id) !== bid) type = "rejected";
      if (ack.source && String(ack.source) !== src) type = "rejected";
      if (ack.session_id && String(ack.session_id) !== String(sid)) type = "rejected";
      if (
        ack.material_generation != null &&
        Number(ack.material_generation) !==
          Number(item && item.material_generation != null ? item.material_generation : 1)
      ) {
        type = "rejected";
      }
    }
    // Only force conflict when server says so (or explicit conflict flags).
    if (type !== "conflict" && (j.conflict === true || j.ack_conflict === true)) {
      type = "conflict";
    }
    var rec = registryGet(bid, src, sid, gen) || {
      batch_id: bid,
      source: src,
      session_id: sid,
      material_generation: gen,
    };
    rec.material_hash = materialHash || rec.material_hash;
    rec.payload_hash = materialHash || rec.payload_hash;
    rec.request_hash = requestHash || rec.request_hash;
    rec.capture_id = (item && item.capture_id) || rec.capture_id;
    var ingest = (j && j.ingest && typeof j.ingest === "object") ? j.ingest : {};
    var durability = String(
      (j && j.durability_state) ||
        (ack && ack.durability_state) ||
        ingest.durability_state ||
        ""
    );
    var coldWritten = j.cold_written;
    if (coldWritten === undefined) coldWritten = ingest.cold_written;
    var coldOk =
      coldWritten !== false ||
      j.cold_skipped_unchanged === true ||
      ingest.cold_skipped_unchanged === true ||
      j.same_capture === true ||
      ingest.same_capture === true ||
      durability === "duplicate_durable";
    if (
      durability === "stored_primary_only" ||
      durability === "accepted_pending_durability" ||
      durability === "failed"
    ) {
      coldOk = false;
    }
    var verified =
      (type === "stored" || type === "duplicate" || type === "merged") && coldOk;
    if (!coldOk && (type === "stored" || type === "duplicate" || type === "merged")) {
      type = "transport_ok_unverified";
    }
    rec.ack_type = type;
    rec.durability_state = durability || (coldOk ? "stored_durable" : "stored_primary_only");
    rec.updated_ms = Date.now();
    try {
      if (
        verified &&
        bid === "B11_interaction" &&
        global.GRRpaMonitor &&
        typeof global.GRRpaMonitor.ackSegment === "function"
      ) {
        global.GRRpaMonitor.ackSegment(
          src,
          (ack && ack.seq_end) || item.rpa_seq_end,
          (ack && ack.segment_id) || item.rpa_segment_id
        );
      }
    } catch (eRpaAck) {}
    if (verified) {
      rec.state = "acked";
      ackStored += type === "stored" ? 1 : 0;
      if (type === "duplicate") ackDuplicate++;
      if (type === "merged") ackMerged++;
    } else if (type === "conflict") {
      rec.state = "conflict";
      ackConflict++;
    } else if (type === "transport_ok_unverified") {
      rec.state = "transport_ok_unverified";
    } else {
      rec.state = "rejected";
    }
    registryPut(rec);
    return { type: type, verified: verified, material_hash: materialHash, request_hash: requestHash };
  }

  /** Deep-clone JSON-safe material so post-freeze collector churn cannot alias nested fields. */
  function deepCloneJson(v) {
    try {
      return JSON.parse(JSON.stringify(v));
    } catch (eDc) {
      return v;
    }
  }

  /**
   * iss/72: freeze capture at boundary — ALL material fields before hash.
   * After freeze, postOne must not mutate item.payload.
   * iss/75: deep-clone fields (shallow Object.assign left nested refs shared with collectors
   * → false `capture_mutated_blocked` on B0/B1/B10 enrich / dual-land).
   */
  function freezeCapture(item) {
    if (!item || item._capture_frozen) return item;
    try {
      var productVer =
        (global.__GR_PRODUCT_VERSION__ ||
          (global.__GR_BOOT__ &&
            (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
          "") + "";
      var payload = item.payload;
      if (!payload || typeof payload !== "object") payload = {};
      else payload = Object.assign({}, payload);
      var pf =
        payload.fields && typeof payload.fields === "object"
          ? deepCloneJson(payload.fields)
          : {};
      if (!pf || typeof pf !== "object") pf = {};
      if (productVer && !pf.product_version) pf.product_version = productVer;
      if (productVer && !payload.product_version) payload.product_version = productVer;
      // Timing once
      try {
        if (pf.t_perf == null) {
          var tPerf =
            typeof performance !== "undefined" && performance.now ? performance.now() : null;
          var tWall = Date.now();
          if (tPerf != null && isFinite(tPerf)) {
            pf.t_perf = Math.round(tPerf * 1000) / 1000;
            pf.t_wall_ms = tWall;
          }
        }
      } catch (eClk) {}
      try {
        if (global.__GR_COMPUTE_PRESSURE_STATE__ && !pf.compute_pressure_state) {
          pf.compute_pressure_state = String(global.__GR_COMPUTE_PRESSURE_STATE__);
        }
      } catch (ePr) {}
      // Cohort / FE impl — frozen INTO material before hash (iss/72 P0-3).
      try {
        if (!pf.cohort_color_gamut && typeof matchMedia === "function") {
          if (matchMedia("(color-gamut: rec2020)").matches) pf.cohort_color_gamut = "rec2020";
          else if (matchMedia("(color-gamut: p3)").matches) pf.cohort_color_gamut = "p3";
          else if (matchMedia("(color-gamut: srgb)").matches) pf.cohort_color_gamut = "srgb";
          else pf.cohort_color_gamut = "unknown";
        }
        if (pf.cohort_dpr_bucket == null && typeof devicePixelRatio === "number") {
          pf.cohort_dpr_bucket = Math.round(devicePixelRatio * 4) / 4;
        }
        if (!pf.cohort_pointer && typeof matchMedia === "function") {
          pf.cohort_pointer = matchMedia("(pointer: fine)").matches
            ? "fine"
            : matchMedia("(pointer: coarse)").matches
              ? "coarse"
              : "none";
        }
        if (!pf.cohort_intl_locale) {
          try {
            pf.cohort_intl_locale = Intl.DateTimeFormat().resolvedOptions().locale || "";
            pf.cohort_intl_calendar = Intl.DateTimeFormat().resolvedOptions().calendar || "";
            pf.cohort_intl_numbering = Intl.DateTimeFormat().resolvedOptions().numberingSystem || "";
          } catch (eIntl) {}
        }
        if (pf.cohort_speech_voices_n == null) {
          try {
            if (typeof speechSynthesis !== "undefined" && speechSynthesis.getVoices) {
              pf.cohort_speech_voices_n = (speechSynthesis.getVoices() || []).length;
            }
          } catch (eVo) {}
        }
        var fePacks = String(global.__GR_FE_PACKS_VERSION__ || global.__GR_FE_CODE_VERSION__ || "");
        if (fePacks) {
          if (!pf.fe_packs_version) pf.fe_packs_version = fePacks;
          if (!pf.fe_code_version) pf.fe_code_version = fePacks;
        }
        // Seal B10 requires fe_impl_version ∈ product epoch allowlist.
        // Prefer server product over sticky module BUILD_IMPL (v5.8.*) stamps.
        try {
          var prodEpoch =
            String(
              global.__GR_SERVER_PRODUCT_VERSION__ ||
                global.__GR_PRODUCT_VERSION__ ||
                (global.__GR_BOOT__ &&
                  (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
                ""
            ) || "";
          if (prodEpoch) {
            pf.fe_impl_version = prodEpoch;
            try {
              global.__GR_FE_IMPL_VERSION__ = prodEpoch;
            } catch (eSet) {}
          } else if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        } catch (eImpl) {
          if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        }
        if (!pf.fe_build_impl && global.__GR_BUILD_IMPL__) {
          pf.fe_build_impl = String(global.__GR_BUILD_IMPL__);
        }
      } catch (eCoh) {}
      payload.fields = pf;
      item.payload = payload;
      if (!item.capture_id) {
        item.capture_id =
          "cap_" +
          String(item.batch_id || "b") +
          "_" +
          String(Date.now()) +
          "_" +
          String(Math.floor(Math.random() * 1e6));
      }
      if (item.material_generation == null) item.material_generation = 1;
      item.collected_at_ms = item.collected_at_ms || Date.now();
      item.payload_hash = payloadHashOf(payload);
      item.material_hash = item.payload_hash;
      item._capture_frozen = true;
      // Deep-freeze snapshot for retry identity checks + restore on post-freeze mutation.
      try {
        item._frozen_fields = deepCloneJson(payload.fields || {});
        item._frozen_payload_json = stableStringify(item._frozen_fields || {});
      } catch (eSnap) {}
      captureFreezeCount++;
      registryUpsertFromItem(item, "collected");
    } catch (eFz) {
      try {
        item._capture_frozen = true;
      } catch (e2) {}
    }
    return item;
  }

  /** Reset queue/registry state when session_id changes (iss/72 P1-2). */
  function resetSessionState(reason) {
    materialRegistry = Object.create(null);
    sessionOutcome = {
      sealed_ok: 0,
      sealed_reject: 0,
      plain_ok: 0,
      network: 0,
      other_fail: 0,
      batches_ok: Object.create(null),
      batches_started: Object.create(null),
    };
    forceRecollectUsed = Object.create(null);
    sentKeys = Object.create(null);
    attempts = Object.create(null);
    outcomeReported = false;
    try {
      global.__GR_RECEIVED_BATCHES__ = [];
    } catch (eR) {}
    // Drop pending from other sessions
    try {
      var keep = [];
      for (var i = 0; i < pending.length; i++) {
        if (pending[i] && String(pending[i].session_id || "") === String(cfg.session_id || "")) {
          keep.push(pending[i]);
        }
      }
      pending = keep;
    } catch (eP) {}
    try {
      if (global.GROps && GROps.report) {
        GROps.report(
          "queue_session_reset",
          "upload",
          { reason: String(reason || "session_change") },
          "info"
        );
      }
    } catch (eO) {}
  }

  /**
   * iss/70 P2: effective upload concurrency = min of budgets (never max-stack).
   */
  function effectiveUploadCap() {
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var hardMax = Math.max(floor, Number(cfg.concurrency_max) || 6);
    var base = Math.max(floor, Number(cfg.concurrency) || 3);
    // Lifecycle: unloading → 1–2; background → low.
    var lifeCap = hardMax;
    try {
      if (isPageUnloading() || hiding || global.__GR_PAGE_UNLOADING__) lifeCap = 2;
      else if (global.__GR_PAGE_BACKGROUNDED__ || isPageBackgrounded()) lifeCap = 2;
    } catch (eL) {}
    // Pressure: serious/critical → floor.
    try {
      var st = String(global.__GR_COMPUTE_PRESSURE_STATE__ || "").toLowerCase();
      if (st === "serious" || st === "critical") lifeCap = Math.min(lifeCap, floor);
    } catch (eP) {}
    // Fail storm: half.
    if (recentUploadFail >= 3 && recentUploadOk < recentUploadFail) {
      lifeCap = Math.min(lifeCap, Math.max(floor, Math.floor(base / 2) || floor));
    }
    return Math.max(floor, Math.min(hardMax, base, lifeCap));
  }

  /** AIMD: success window +1 (cap max); fail → half. */
  function aimdOnSuccess() {
    recentUploadOk++;
    recentUploadFail = Math.max(0, recentUploadFail - 1);
    var maxC = Math.max(1, Number(cfg.concurrency_max) || 6);
    if (recentUploadOk >= 3 && recentUploadFail === 0) {
      var next = Math.min(maxC, (Number(cfg.concurrency) || 3) + 1);
      if (next > cfg.concurrency) {
        cfg.concurrency = next;
        aimdUps++;
      }
      recentUploadOk = 0;
    }
  }

  function aimdOnFail() {
    recentUploadFail++;
    recentUploadOk = 0;
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var cur = Number(cfg.concurrency) || 3;
    var next = Math.max(floor, Math.floor(cur / 2) || floor);
    if (next < cur) {
      cfg.concurrency = next;
      aimdDowns++;
    }
  }

  /** True if this batch has a capture in pending/retry/upload (not yet batches_ok). */
  function hasPendingCapture(batchId, sessionId) {
    var bid = String(batchId || "");
    if (!bid) return false;
    var sid = sessionId != null ? String(sessionId) : "";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      return true;
    }
    var i;
    for (i = 0; i < pending.length; i++) if (match(pending[i])) return true;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return true;
    return false;
  }

  /**
   * Material/transport state for hard-SLA, registry, and tests.
   * acked | conflict | uploading | retry_wait | queued | collected | absent
   */
  function materialState(batchId, sessionId, source, generation) {
    var bid = String(batchId || "");
    if (!bid) return "absent";
    if (batchAlreadyOk(bid)) return "acked";
    var sid = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
    var src = source || "main";
    var gen = generation != null ? generation : 1;
    var rec = registryGet(bid, src, sid, gen);
    if (rec && rec.state === "acked") return "acked";
    if (rec && rec.state === "conflict") return "conflict";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      if (src && it.source && String(it.source) !== String(src)) return false;
      return true;
    }
    var i;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return "uploading";
    for (i = 0; i < pending.length; i++) {
      if (!match(pending[i])) continue;
      if (pending[i]._retry_after_ms && pending[i]._retry_after_ms > Date.now()) return "retry_wait";
      return "queued";
    }
    if (rec && rec.state) return rec.state;
    return "absent";
  }

  function removeInflightItem(item) {
    var kWant = "";
    try {
      kWant = keyOf(item);
    } catch (eK) {
      kWant = "";
    }
    var removed = 0;
    for (var i = inflightItems.length - 1; i >= 0; i--) {
      var cur = inflightItems[i];
      if (cur === item) {
        inflightItems.splice(i, 1);
        removed++;
        continue;
      }
      // Identity can diverge after freeze/clone — also drop by material key.
      try {
        if (kWant && keyOf(cur) === kWant) {
          inflightItems.splice(i, 1);
          removed++;
        }
      } catch (e2) {}
    }
    // Own the counter: callers must not also inflight-- after this (was double-
    // decrementing → negative inflight under multi-tab / multi-key prune).
    if (removed > 0) inflight = Math.max(0, inflight - removed);
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (inflight < 0) inflight = 0;
  }

  /** Drop stuck inflight slots so multi-tab never accumulates zombie transport rows. */
  function pruneStaleInflight() {
    var cap = Math.max(Number(cfg.concurrency_max) || 6, Number(cfg.concurrency) || 3) * 2;
    if (inflightItems.length <= cap && inflight <= cap) return;
    // Hard clamp: keep newest cap items, drop older zombies.
    if (inflightItems.length > cap) {
      inflightItems = inflightItems.slice(-cap);
    }
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (heavyInflight > 1) heavyInflight = 1;
  }

  function isB10xMustLand(batchId) {
    var id = String(batchId || "");
    for (var i = 0; i < B10X_MUST.length; i++) if (B10X_MUST[i] === id) return true;
    return false;
  }

  /**
   * pagehide / unload allowlist — only mint-critical + secondary silicon.
   * Mid/dense/R/B11 are dropped so keepalive window is not wasted (iss/65).
   */
  function isPagehideAllowlisted(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    if (id.indexOf("B10x_silicon_") === 0) {
      // On unload: prefer must-land; deep only if must-land already ok or nothing else pending.
      if (id === "B10x_silicon_deep") {
        if (!isPageUnloading()) return true;
        var mustPending = false;
        for (var i = 0; i < B10X_MUST.length; i++) {
          if (!batchAlreadyOk(B10X_MUST[i])) {
            // still need must — allow deep only if already inflight material not required
            mustPending = true;
            break;
          }
        }
        // If must still missing, drop deep on pagehide (save slots for ulp/rint/noderiv).
        return !mustPending;
      }
      return true;
    }
    if (
      id === "B0_bootstrap" ||
      id === "B2_hardware" ||
      id === "B3_system" ||
      id === "B1_conflict" ||
      id === "B12_anti_camouflage" ||
      id === "B8_gateway" ||
      id === "B8_gateway_early" ||
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B7_sandbox"
    ) {
      return true;
    }
    return false;
  }

  /**
   * Pagehide / flush priority: commercial hard + secondary silicon (B10x/B47/B18).
   * NOTE: do NOT use this alone for ops severity — B10x/B47 network exhaust must
   * report as deepen/soft exhausted (warn), not upload_hard_exhausted (error).
   * Prod v150: B10x_deep / B47 mis-tagged as hard_exhausted on Failed to fetch.
   */
  function isHardAnchorBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    // Secondary silicon/infra: hard *priority* on pagehide only (not always-hard).
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    ) {
      return isPageUnloading() || !!sessionOutcome.batches_ok["B10_hw_curves"];
    }
    // B10x: hard on pagehide / when B10 already ok (short-visit residual path)
    if (id.indexOf("B10x_silicon_") === 0) {
      if (isPageUnloading()) return true;
      try {
        if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) return true;
      } catch (eB) {}
      return false;
    }
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    // alias mid.curves / primary silicon residual only
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    return false;
  }

  /**
   * Ops severity gate: only true commercial anchors → upload_hard_exhausted error.
   * Silicon deepen / SAB / WebGPU network storms → deepen_exhausted warn.
   */
  function isCommercialHardForOps(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return false;
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B15_cross_curves"
    ) {
      return false;
    }
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    return false;
  }

  function isDeepenBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isDeepenBatch) {
        return !!GRProbeLifecycle.isDeepenBatch(batchId);
      }
    } catch (eD) {}
    var id = String(batchId || "");
    return id.indexOf("B10x_") === 0 || id === "B15_cross_curves";
  }

  function isHeavyBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isHeavyBatch) {
        return !!GRProbeLifecycle.isHeavyBatch(batchId);
      }
    } catch (eH) {}
    var id = String(batchId || "");
    return (
      isDeepenBatch(id) ||
      id === "B7_sandbox" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep"
    );
  }

  function heavyMaxInflight() {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.heavyMaxInflight) {
        return Math.max(1, GRProbeLifecycle.heavyMaxInflight() | 0);
      }
      if (global.GRProbeLifecycle && GRProbeLifecycle.POLICY) {
        return Math.max(1, (GRProbeLifecycle.POLICY.heavy_max_inflight || 1) | 0);
      }
    } catch (eM) {}
    return 1;
  }

  function isStartHeartbeatItem(item) {
    try {
      var f =
        (item && item.payload && item.payload.fields) ||
        (item && item.fields) ||
        null;
      if (!f || typeof f !== "object") return false;
      if (String(f.b10x_phase || "") === "start") return true;
      if (String(f.b10x_err || "") === "started") return true;
      if (String(f.b10x_err || "") === "helpers_missing") return true;
      return false;
    } catch (e) {
      return false;
    }
  }

  function noteOutcome(kind, batchId, item) {
    try {
      if (kind === "sealed_ok") sessionOutcome.sealed_ok++;
      else if (kind === "sealed_reject") sessionOutcome.sealed_reject++;
      else if (kind === "plain_ok") sessionOutcome.plain_ok++;
      else if (kind === "network") sessionOutcome.network++;
      else sessionOutcome.other_fail++;
      if (batchId && (kind === "sealed_ok" || kind === "plain_ok")) {
        var bid = String(batchId);
        // Start/heartbeat must not count as terminal success (blocks final multipath).
        if (isStartHeartbeatItem(item)) {
          sessionOutcome.batches_started[bid] = 1;
          return;
        }
        sessionOutcome.batches_ok[bid] = 1;
        // After B10 lands, boost pending B10x so residual path wins short dwell.
        if (bid === "B10_hw_curves") {
          try {
            boostPendingB10x();
          } catch (eBoost) {}
        }
      }
    } catch (eO) {}
  }

  function boostPendingB10x() {
    var boost = Number(cfg.b10x_post_b10_boost || 1500);
    for (var i = 0; i < pending.length; i++) {
      var it = pending[i];
      if (!it) continue;
      var bid = String(it.batch_id || "");
      if (bid.indexOf("B10x_silicon_") === 0) {
        it.priority = Math.max(Number(it.priority || 0), boost);
      }
    }
    try {
      pending.sort(function (a, b) {
        return Number(b.priority || 0) - Number(a.priority || 0);
      });
    } catch (eSrt) {}
    pump();
  }

  /** P2: one session summary for ops (gateway_only vs browser_probe). */
  function reportSessionUploadSummary(force) {
    if (outcomeReported && !force) return;
    try {
      var okKeys = Object.keys(sessionOutcome.batches_ok || {});
      var hasB0 = !!sessionOutcome.batches_ok["B0_bootstrap"];
      var hasB10 = !!sessionOutcome.batches_ok["B10_hw_curves"];
      var hasB8 =
        !!sessionOutcome.batches_ok["B8_gateway"] ||
        !!sessionOutcome.batches_ok["B8_gateway_early"];
      var hasB10x = false;
      for (var k = 0; k < okKeys.length; k++) {
        if (String(okKeys[k]).indexOf("B10x_silicon_") === 0) {
          hasB10x = true;
          break;
        }
      }
      var onlyGw = hasB8 && !hasB0 && okKeys.length <= 2;
      var depthClass = onlyGw
        ? "gateway_only"
        : hasB0 && hasB10 && hasB10x
          ? "browser_b0_b10_silicon"
          : hasB0 && hasB10
            ? "browser_b0_b10"
            : hasB0
              ? "browser_partial"
              : okKeys.length
                ? "browser_lite"
                : "empty";
      if (global.GROps && GROps.report) {
        outcomeReported = true;
        GROps.report(
          "upload_session_summary",
          "upload",
          {
            sealed_ok: sessionOutcome.sealed_ok,
            sealed_reject: sessionOutcome.sealed_reject,
            plain_ok: sessionOutcome.plain_ok,
            network: sessionOutcome.network,
            other_fail: sessionOutcome.other_fail,
            batches_ok_n: okKeys.length,
            has_b0: hasB0,
            has_b10: hasB10,
            has_b10x: hasB10x,
            probe_depth_class: depthClass,
          },
          "info"
        );
      }
    } catch (eS) {}
  }

  /** True only on real unload — tab background must NOT set this. */
  function isPageUnloading() {
    try {
      return !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_UNLOADING__);
    } catch (e) {
      return false;
    }
  }

  function isPageBackgrounded() {
    try {
      return !!global.__GR_PAGE_BACKGROUNDED__;
    } catch (e) {
      return false;
    }
  }

  function effectivePriority(item) {
    var p = item.priority || 0;
    var bid = String((item && item.batch_id) || "");
    // Always prefer commercial land + early caps over soft-pack storms (iss/75 texture race).
    if (bid === "B10_hw_curves" || bid === "mid.curves") p += 2200;
    else if (bid === "B2_hardware") p += 1800;
    else if (isCommercialLandFast(bid)) p += 1600;
    if (hiding && isHardAnchorBatch(bid)) {
      p += cfg.hard_anchor_priority_boost || 1000;
    }
    // Active maximize path (alive dwell): keep secondary silicon/infra + B10x above soft
    // media force storms (B19 dense hedge / client_hints). Aligns with probe_field_priority
    // silicon_p0 → media_p1/infra_p1 without blocking soft packs from eventually uploading.
    // Observed: Brave matrix dwell ended pending=250, B19×35, missing B47/B18/B46.
    if (!hiding && !isPageUnloading()) {
      var b10Ok = false;
      try {
        b10Ok = !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]);
      } catch (eB10) {}
      if (isB10xBatch(bid)) {
        p += b10Ok ? cfg.b10x_post_b10_boost || 1500 : 900;
      } else if (isSecondaryInfraBatch(bid)) {
        // B47/B18/B46/B10x_deep: must not sit behind soft force re-queues mid-dwell.
        p += b10Ok ? 1200 : 700;
      }
    }
    // Unload: must-land B10x above deep / secondary so short keepalive lands silicon.
    if (hiding || isPageUnloading()) {
      if (isB10xMustLand(bid)) p += 2500;
      else if (bid === "B10_hw_curves" || bid === "mid.curves") p += 3000;
      else if (bid === "B10x_silicon_deep") p -= 400;
      else if (!isPagehideAllowlisted(bid)) p -= 5000;
    }
    return p;
  }

  /**
   * Insert or replace pending by session|batch|source key.
   * Force items MUST coalesce (was: force=true skipped replace → N copies of B19/secondary).
   * Does not change wire semantics: still one transport path per key at a time.
   * @returns {"pushed"|"replaced"|"dropped"}
   */
  function queuePending(item) {
    if (!item || !item.batch_id) return "dropped";
    var k = keyOf(item);
    var genNew = (item.material_generation || 1) | 0;
    for (var i = 0; i < pending.length; i++) {
      if (keyOf(pending[i]) !== k) continue;
      var cur = pending[i];
      var genCur = (cur.material_generation || 1) | 0;
      // Never demote a higher material generation already queued.
      if (genNew < genCur) {
        skipped++;
        return "dropped";
      }
      // Same key: replace with fresher capture (force or not). Counts as dedupe.
      pending[i] = item;
      skipped++;
      return "replaced";
    }
    pending.push(item);
    return "pushed";
  }

  function keyOf(item) {
    return [item.session_id || cfg.session_id, item.batch_id, item.source || "main"].join("|");
  }

  function sessionOf(item) {
    return String(
      (item && item.session_id) || cfg.session_id || global.__GR_SESSION_ID__ || ""
    );
  }

  /** Batches that must stop when cycle is complete / cool (identity path). */
  function isIdentityBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return true;
    // Page RPA after cool still uses B11 in some designs — block on hard halt too
    // because server require_active_session rejects ALL ingest on complete cycle.
    return true;
  }

  /**
   * Terminal = this cycle must stop identity uploads.
   * Align with BE:
   *   - 410 / cycle_closed / halt_uploads / analysis_terminal / cycle_complete → halt
   *   - soft probe_complete alone → NOT halt (still may upload rest packs)
   *   - open cool skip_identity → halt identity
   */
  /**
   * Schedule-final complete (maximize probe policy v5.8.53+).
   * True only when brain schedule is done AND silicon/B10 materials exist.
   * commercial_identity_final / dh_+curves alone is a milestone — NOT final.
   */
  function hardFinalComplete(j, cps) {
    cps = cps || {};
    var cov = cps.identity_coverage || j.identity_coverage || {};
    // EDH: block cool while silicon B10x still required/missing.
    try {
      var edh = j.edh || j.device_hypothesis || {};
      var rg = edh.research_gate || {};
      if (rg.b10x_required === true && rg.b10x_complete === false) return false;
      if (j.b10x_must_land === true) return false;
      var miss = (j.route_plan && j.route_plan.b10x_missing) || (j.b10x_missing) || [];
      if (miss && miss.length) return false;
      var rp = j.route_plan || (j.brain && j.brain.route_plan) || {};
      var packs = rp.packs || [];
      for (var pi = 0; pi < packs.length; pi++) {
        var pid = String((packs[pi] && (packs[pi].pack_id || packs[pi].id)) || "");
        if (pid.indexOf("B10x_silicon_") === 0) return false;
      }
    } catch (eEdh) {}

    // Primary residual B10 MUST land before any cool/final — do not trust digest alone.
    var ids = cps.received_batch_ids || j.received_batch_ids || [];
    var hasB10 = false;
    for (var i = 0; i < ids.length; i++) {
      var id = typeof ids[i] === "string" ? ids[i] : ids[i] && ids[i].batch_id;
      if (id === "B10_hw_curves" || id === "mid.curves") {
        hasB10 = true;
        break;
      }
    }
    try {
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) hasB10 = true;
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]) hasB10 = true;
    } catch (eLoc) {}
    try {
      if (j.b10_present === true && (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"])) {
        hasB10 = true;
      }
    } catch (eB) {}
    // Without primary B10, never final (keeps self-heal / upload alive).
    if (!hasB10) return false;

    // commercial_identity_final alone → NOT final (continue soft/mid packs).
    var coverageComplete =
      cov.coverage_complete === true ||
      cps.coverage_complete === true ||
      j.coverage_complete === true ||
      (j.coverage && j.coverage.coverage_complete === true) ||
      (j.brain && j.brain.coverage && j.brain.coverage.coverage_complete === true);
    var stopProbe =
      j.stop_probe === true ||
      (j.route_plan && j.route_plan.stop_probe === true) ||
      (cps.probe_complete === true && coverageComplete);
    var brainTerm =
      j.analysis_terminal === true ||
      cps.analysis_terminal === true ||
      (j.analysis && j.analysis.analysis_terminal === true) ||
      (stopProbe && coverageComplete);
    var closed =
      cps.cycle_status === "complete" ||
      j.cycle_complete === true ||
      cps.cycle_closed === true ||
      j.cycle_closes === true ||
      (j.analysis && (j.analysis.cycle_closes === true || j.analysis.cycle_complete === true));
    var scheduleFinal =
      cov.brain_schedule_final === true ||
      cps.brain_schedule_final === true ||
      j.brain_schedule_final === true ||
      cov.final_analysis_ok === true ||
      cps.final_analysis_ok === true ||
      j.final_analysis_ok === true;

    // Final = primary B10 landed AND (schedule done OR closed OR hard_complete).
    if (scheduleFinal) return true;
    if (brainTerm || closed) return true;
    if (cov.hard_complete === true) return true;
    return false;
  }

  function parseTerminal(status, j) {
    j = j || {};
    var err = j.error || j.code || j.expired_reason || "";
    var code = j.code || j.expired_reason || "";
    var cps = j.cycle_probe_status || {};
    var analysis = j.analysis || {};
    var s =
      String(err) +
      " " +
      String(code) +
      " " +
      String(cps.cycle_status || "") +
      " " +
      String(cps.business_state || "") +
      " " +
      String(cps.expired_reason || "");

    // Soft cycle close (v5.8.124): HTTP 200 + accepted:false + halt_uploads.
    // Preferred over 410 so browser Network is not full of red errors.
    if (
      status === 200 &&
      j &&
      (j.accepted === false ||
        j.identity_accepted === false ||
        (j.halt_uploads === true &&
          (j.cycle_closed === true ||
            j.code === "cycle_complete" ||
            j.code === "cycle_purged" ||
            j.code === "incomplete_ttl" ||
            j.code === "session_expired" ||
            (j.cycle_probe_status && j.cycle_probe_status.http_soft_close))))
    ) {
      return {
        terminal: true,
        code: code || j.code || cps.expired_reason || "cycle_complete",
        status: 200,
        business_state: j.business_state || cps.business_state || "identity_complete_cool",
        soft_close: true,
      };
    }

    // Legacy HTTP 410: cycle no longer accepts identity ingest (complete|purged|ttl).
    if (status === 410) {
      return {
        terminal: true,
        code: code || cps.expired_reason || "session_expired",
        status: 410,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }

    // Authoritative hard+final complete (probe completeness + final analysis).
    if (hardFinalComplete(j, cps)) {
      return {
        terminal: true,
        code: code || "identity_final_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // halt_uploads from server only if hard complete OR cycle truly closed with hard materials.
    if (j.halt_uploads === true || cps.halt_uploads === true || analysis.halt_uploads === true) {
      if (!hardFinalComplete(j, cps) && cps.cycle_status !== "purged" && status !== 410) {
        // Thin halt flag — keep uploading B10 (server may lag; hard SLA continues).
        return { terminal: false };
      }
      var haltCode =
        code ||
        (analysis.cycle_complete || j.cycle_complete
          ? "cycle_complete"
          : cps.expired_reason || cps.cycle_status || cps.business_state || "halt_uploads");
      return {
        terminal: true,
        code: haltCode,
        status: status || 200,
        business_state: cps.business_state || "halt_uploads",
      };
    }

    // Cycle closed flags — only halt when hard materials present (or purged).
    if (
      j.cycle_closes === true ||
      j.cycle_complete ||
      cps.cycle_closed === true ||
      cps.cycle_status === "complete" ||
      cps.cycle_status === "purged" ||
      analysis.cycle_complete ||
      analysis.cycle_closes === true
    ) {
      if (cps.cycle_status === "purged") {
        return {
          terminal: true,
          code: "cycle_purged",
          status: status || 200,
          business_state: "purged",
        };
      }
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cycle_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // Soft analysis_terminal without B10: keep probing (do not halt).
    if (
      j.analysis_terminal === true ||
      analysis.analysis_terminal === true ||
      cps.analysis_terminal === true
    ) {
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: "analysis_terminal",
        status: status || 200,
        business_state: "analysis_terminal",
      };
    }

    // Cool / open skip identity — ONLY when phase/business_state says cool AND silicon ok.
    if (
      j.phase === "cool" ||
      cps.business_state === "identity_complete_cool" ||
      (j.skip_identity_probe === true &&
        (j.phase === "cool" ||
          cps.cycle_status === "complete" ||
          cps.halt_uploads === true ||
          j.halt_uploads === true)) ||
      (j.skip_session_probe === true &&
        (j.phase === "cool" || cps.business_state === "identity_complete_cool"))
    ) {
      if (j.cool_silicon_ok === false || cps.cool_silicon_ok === false) {
        return { terminal: false };
      }
      // Prefer hard materials when available; thin cool already blocked server-side.
      if (!hardFinalComplete(j, cps) && j.cool_silicon_ok !== true && cps.has_b10 !== true) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cool",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    if (/session_expired|cycle_purged|incomplete_ttl/i.test(s)) {
      return {
        terminal: true,
        code: code || "session_expired",
        status: status || 0,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }
    // cycle_complete / halt_uploads in error string alone must not stop thin uploads.
    return { terminal: false };
  }

  function maybeLocalIdentityDone() {
    // Local channel only: FE-side upload progress. Does NOT close the cycle —
    // server analysis_terminal / complete_cycle / 410 remain authoritative for halt.
    var need = cfg.hard_anchor_batches || [];
    var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
    var ok = 0;
    var present = [];
    for (var i = 0; i < need.length; i++) {
      var bid = need[i];
      if (bid === "B8_gateway_early") continue;
      var k1 = [sid, bid, "main"].join("|");
      var k2 = bid === "B8_gateway" ? [sid, "B8_gateway_early", "main"].join("|") : null;
      if (sentKeys[k1] || (k2 && sentKeys[k2])) {
        ok++;
        present.push(bid);
      }
    }
    var idle = pending.length === 0 && inflight <= 1;
    // B0+B1+B2+B3+B12+B8 ≈ 6 without B10 is enough for "lite wave uploaded" local signal
    if (ok >= 5 && idle) {
      try {
        global.__GR_IDENTITY_UPLOADS_DONE__ = {
          at_ms: Date.now(),
          hard_sent: ok,
          present: present,
          session_id: sid,
          // Soft local progress — not cycle closed
          local_only: true,
          cycle_closed: !!(haltState || global.__GR_CYCLE_CLOSED__),
        };
        global.dispatchEvent(
          new CustomEvent("gr-identity-uploads-done", {
            detail: global.__GR_IDENTITY_UPLOADS_DONE__,
          })
        );
      } catch (eD) {}
    }
    // Queue fully idle: notify multi-tick / observers (not halt).
    if (pending.length === 0 && inflight === 0) {
      try {
        global.__GR_UPLOAD_QUEUE_IDLE__ = {
          at_ms: Date.now(),
          sent: sent,
          session_id: sid,
          halted: !!haltState,
        };
        global.dispatchEvent(
          new CustomEvent("gr-upload-queue-idle", {
            detail: global.__GR_UPLOAD_QUEUE_IDLE__,
          })
        );
      } catch (eI) {}
    }
  }

  function phaseForCode(code, reason) {
    var c = String(code || reason || "");
    if (/incomplete_ttl|purged|cycle_purged/i.test(c)) return "expired";
    if (/cool|cycle_complete|analysis_terminal|halt|probe_complete|stop_probe/i.test(c)) return "cool";
    return "cool";
  }

  function applyHalt(reason, sessionId, code) {
    var sid = String(sessionId || cfg.session_id || global.__GR_SESSION_ID__ || "");
    var c = code || reason || "halt";
    // Idempotent: same session already halted — still abort leftovers.
    try {
      reportSessionUploadSummary(true);
    } catch (eSum) {}
    var cLow0 = String(c || reason || "").toLowerCase();
    // Soft-close preferred: cycle complete / cool are product-normal (HTTP 200), not 410.
    var softTerminal =
      cLow0.indexOf("cycle_complete") >= 0 ||
      cLow0.indexOf("identity_complete") >= 0 ||
      cLow0.indexOf("identity_final") >= 0 ||
      cLow0 === "cool" ||
      cLow0 === "probe_complete" ||
      cLow0 === "analysis_terminal" ||
      cLow0 === "incomplete_ttl" ||
      cLow0 === "cycle_purged" ||
      cLow0 === "session_expired";
    haltState = {
      reason: reason || "halt",
      session_id: sid,
      code: c,
      at_ms: Date.now(),
      business_state: phaseForCode(c, reason),
      status: softTerminal ? 200 : 410,
      soft_close: !!softTerminal,
    };
    try {
      global.__GR_STOP_PROBE__ = true;
      global.__GR_SKIP_IDENTITY__ = true;
      global.__GR_HALT_UPLOADS__ = true;
      global.__GR_PHASE__ = phaseForCode(c, reason);
      global.__GR_UPLOAD_HALT__ = haltState;
      global.__GR_CYCLE_CLOSED__ = {
        session_id: sid,
        code: c,
        reason: reason,
        at_ms: haltState.at_ms,
        soft_close: !!softTerminal,
      };
      // Persist cool ONLY on true schedule-final / identity_complete / cycle_complete cool.
      // Cool is version-scoped (product_version stamp); version change invalidates via storage.
      // Do NOT stamp 24h cool on generic superseded/pagehide — that froze incomplete VT gap-fill.
      try {
        var S = global.GRStorage;
        var cLow = String(c || reason || "").toLowerCase();
        var coolOkHalt =
          cLow.indexOf("identity_complete") >= 0 ||
          cLow.indexOf("identity_final") >= 0 ||
          cLow.indexOf("schedule_final") >= 0 ||
          cLow.indexOf("cycle_complete") >= 0 ||
          cLow === "cool" ||
          cLow === "probe_complete" ||
          cLow === "analysis_terminal" ||
          !!global.__GR_BRAIN_SCHEDULE_FINAL__ ||
          !!global.__GR_HARD_FINAL__;
        // pagehide / superseded without schedule final → no cool stamp
        if (
          S &&
          S.setCoolUntil &&
          coolOkHalt &&
          !isPageUnloading()
        ) {
          var until = S.getCoolUntil && S.getCoolUntil();
          var now = Date.now();
          var ver =
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            (global.__GR_BOOT__ && (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
            global.__GR_PRODUCT_VERSION__ ||
            (haltState && haltState.product_version) ||
            undefined;
          if (!until || until <= now) {
            S.setCoolUntil(now + 24 * 60 * 60 * 1000, ver);
          } else {
            // Refresh version stamp on existing cool window (keeps cool tied to current version).
            S.setCoolUntil(until, ver);
          }
        }
      } catch (eCool) {}
    } catch (eG) {}
    // Drop pending except B10x EDH must-land (and explicit force_after_halt).
    if (pending.length) {
      var keepB10x = [];
      for (var pi = 0; pi < pending.length; pi++) {
        if (allowDespiteHalt(pending[pi])) keepB10x.push(pending[pi]);
        else haltedDrops++;
      }
      pending = keepB10x;
    }
    if (!pending.length) {
      cfg.concurrency = 0;
    } else {
      // Keep a small pump capacity so silicon B10x can finish after identity halt.
      if (cfg.concurrency < 2) cfg.concurrency = 2;
      try {
        setTimeout(function () {
          pump();
        }, 0);
      } catch (eP) {}
    }
    // Abort in-flight fetch so concurrent POSTs do not all land as 410 in Network.
    var ctrls = inflightCtrls.slice();
    inflightCtrls = [];
    for (var ai = 0; ai < ctrls.length; ai++) {
      try {
        if (ctrls[ai] && ctrls[ai].abort) ctrls[ai].abort();
      } catch (eAb) {}
    }
    try {
      global.dispatchEvent(
        new CustomEvent("gr-cycle-closed", {
          detail: {
            reason: haltState.reason,
            code: haltState.code,
            session_id: haltState.session_id,
          },
        })
      );
    } catch (eEv) {}
  }

  function isHaltedFor(item) {
    try {
      if (global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) return true;
    } catch (eH) {}
    if (!haltState) return false;
    var sid = sessionOf(item);
    if (!haltState.session_id) return true;
    if (!sid) return true;
    return sid === haltState.session_id;
  }

  /** EDH silicon deepen must still upload after identity halt / cool. */
  function isB10xBatch(batchId) {
    return String(batchId || "").indexOf("B10x_") === 0;
  }

  /** iss/54–57 secondary silicon/infra — must land ok or honest skip even after identity halt. */
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
   * Re-open cycle to remint seal_grant (open lag / wiped grant).
   * Module-level so enqueue + postOne share one coalesced refresh.
   */
  function refreshSealGrant(sid) {
    if (global.__GR_SEAL_REFRESH_P__) return global.__GR_SEAL_REFRESH_P__;
    var base = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "");
    var cycle = String(
      sid ||
        global.__GR_SESSION_ID__ ||
        global.__GR_CYCLE_ID__ ||
        cfg.session_id ||
        ""
    );
    var vt = "";
    try {
      vt =
        global.__GR_VTID__ ||
        (global.GRStorage && GRStorage.visitorTerminalId && GRStorage.visitorTerminalId()) ||
        "";
    } catch (eV) {}
    var site =
      global.__GR_SITE_ID__ ||
      (global.__GR_BOOT__ && (global.__GR_BOOT__.site_id || global.__GR_BOOT__.siteId)) ||
      "";
    // embed gate: open 需带站点 embed_token。优先 __GR_BOOT__.embed_token，
    // 否则从 pin_url / 含 grt= 的 script 标签取(与 gr.boot embedTokenFromUrl 同源)。
    var etok = "";
    try {
      var bc = global.__GR_BOOT__ || {};
      etok = String(bc.embed_token || "");
      if (!etok) {
        var srcs = [];
        if (bc.pin_url) srcs.push(String(bc.pin_url));
        try {
          if (typeof document !== "undefined" && document.querySelectorAll) {
            var tags = document.querySelectorAll('script[src*="grt="]');
            for (var i = 0; i < tags.length; i++) srcs.push(String(tags[i].src || ""));
          }
        } catch (eQs0) {}
        for (var j = 0; j < srcs.length; j++) {
          var m = srcs[j].match(/[?&]grt=([^&]+)/);
          if (m) {
            etok = decodeURIComponent(m[1]);
            break;
          }
        }
      }
    } catch (eEt) {}
    var openUrl = base + "/v1/session/open";
    var openSame = false;
    try {
      openSame =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(openUrl, location.href).origin === location.origin;
    } catch (eOpenSo) {
      openSame = String(openUrl).charAt(0) === "/";
    }
    global.__GR_SEAL_REFRESH_P__ = fetch(openUrl, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        cycle_id: cycle || undefined,
        session_id: cycle || undefined,
        visitor_terminal_id: vt || undefined,
        site_id: site || undefined,
        embed_token: etok || undefined,
        meta: { fe: "seal_refresh", inject_path: cfg.inject_path || "app" },
      }),
      // CF orange: same-origin must send cf_clearance; cross-origin include when CORS allows
      credentials: openSame ? "same-origin" : "include",
      mode: openSame ? "same-origin" : "cors",
      cache: "no-store",
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        if (j && j.seal_grant) {
          try {
            global.__GR_SEAL_GRANT__ = j.seal_grant;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.seal_grant = j.seal_grant;
            if (j.require_sealed_ingest) {
              global.__GR_REQUIRE_SEALED__ = true;
              global.__GR_BOOT__.require_sealed_ingest = true;
            }
            if (global.GRSeal) {
              if (GRSeal.setRequireSealed && j.require_sealed_ingest) GRSeal.setRequireSealed(true);
              if (GRSeal.setGrant) GRSeal.setGrant(j.seal_grant);
              if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            }
            var ns = j.cycle_id || j.session_id || "";
            if (ns) {
              global.__GR_SESSION_ID__ = ns;
              global.__GR_CYCLE_ID__ = ns;
              cfg.session_id = ns;
            }
          } catch (eG) {}
          return true;
        }
        return false;
      })
      .catch(function () {
        return false;
      })
      .then(function (ok) {
        global.__GR_SEAL_REFRESH_P__ = null;
        try {
          if (ok) pump();
        } catch (eK) {}
        return ok;
      });
    return global.__GR_SEAL_REFRESH_P__;
  }

  function allowDespiteHalt(item) {
    if (!item) return false;
    if (item.force_after_halt || item.allow_during_stop) return true;
    if (isB10xBatch(item.batch_id)) return true;
    if (isSecondaryInfraBatch(item.batch_id)) return true;
    return false;
  }

  function uploadsBlocked(item) {
    if (allowDespiteHalt(item)) return false;
    if (haltState || (function () {
      try {
        return !!(global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__);
      } catch (e) {
        return false;
      }
    })()) {
      return true;
    }
    return isHaltedFor(item || {});
  }

  /** True when uploads go same-origin first-party (/g5) — lower handshake cost, no CORS preflight. */
  function isFirstPartyMode() {
    try {
      if (global.__GR_FIRST_PARTY__) return true;
      var b = global.__GR_BOOT__ || {};
      if (b.first_party || b.firstParty) return true;
      var raw = String(cfg.apiBase || b.apiBase || "").trim();
      if (raw.charAt(0) === "/") return true;
      if (raw && typeof location !== "undefined" && location.origin) {
        return new URL(raw, location.href).origin === location.origin;
      }
    } catch (eFp) {}
    return false;
  }

  /** Resolve relative first-party apiBase (/g5) against page origin. */
  function resolvedApiBase() {
    var raw = String(cfg.apiBase || global.__GR_BOOT__ && global.__GR_BOOT__.apiBase || "").trim();
    if (!raw && (global.__GR_FIRST_PARTY__ || (global.__GR_BOOT__ && global.__GR_BOOT__.first_party))) {
      raw = "/g5";
    }
    if (!raw) return "";
    if (/^https?:\/\//i.test(raw)) return raw.replace(/\/$/, "");
    if (raw.indexOf("//") === 0) {
      try {
        return (location.protocol + raw).replace(/\/$/, "");
      } catch (e0) {
        return ("https:" + raw).replace(/\/$/, "");
      }
    }
    if (raw.charAt(0) !== "/") raw = "/" + raw;
    raw = raw.replace(/\/$/, "");
    try {
      return (location.origin + raw).replace(/\/$/, "");
    } catch (e1) {
      return raw;
    }
  }

  /**
   * iss/70 P2: first-party may raise *caps*, never force high floor via max-stack.
   * effectiveUploadCap() is the final gate.
   */
  function applyFirstPartyPerfHints() {
    if (!isFirstPartyMode()) return;
    try {
      var mc = Number(global.__GR_UPLOAD_CONCURRENCY__ || 0);
      var mr = Number(global.__GR_MID_RAMP_CONCURRENCY__ || 0);
      var man =
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null;
      var bh = (man && man.brain_hints) || {};
      var maxC = Number(cfg.concurrency_max) || 6;
      // Requests may raise concurrency_max, not stamp concurrency to 12/16.
      if (mc > 0) cfg.concurrency_max = Math.min(8, Math.max(maxC, Math.min(mc, 8)));
      if (mr > 0) cfg.mid_ramp_concurrency = Math.min(cfg.concurrency_max, Math.max(cfg.mid_ramp_concurrency || 0, Math.min(mr, 6)));
      if (bh.upload_concurrency != null) {
        var req = Number(bh.upload_concurrency) || 0;
        if (req > 0) cfg.concurrency_max = Math.min(8, Math.max(cfg.concurrency_max || 6, Math.min(req, 8)));
      }
      if (bh.mid_ramp_concurrency != null) {
        var reqM = Number(bh.mid_ramp_concurrency) || 0;
        if (reqM > 0) {
          cfg.mid_ramp_concurrency = Math.min(
            cfg.concurrency_max || 6,
            Math.max(cfg.mid_ramp_concurrency || 0, Math.min(reqM, 6))
          );
        }
      }
      // Keep base concurrency at floor..max, never inflate to 12+.
      cfg.concurrency = Math.min(
        cfg.concurrency_max || 6,
        Math.max(cfg.concurrency_floor || 1, Number(cfg.concurrency) || 3)
      );
    } catch (eMan) {}
  }

  function postOne(item) {
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var base = resolvedApiBase() || String(cfg.apiBase || "").replace(/\/$/, "");
    var url = base + "/v1/ingest";
    var productVer =
      (global.__GR_PRODUCT_VERSION__ ||
        (global.__GR_BOOT__ &&
          (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
        "") + "";
    // iss/72: freeze once at capture boundary; never mutate material after freeze.
    freezeCapture(item);
    var payload = item.payload || {};
    // Integrity: frozen wire material must match capture; restore if collector aliased.
    try {
      if (item._capture_frozen && item._frozen_payload_json) {
        var nowS = stableStringify((payload && payload.fields) || {});
        if (nowS !== item._frozen_payload_json) {
          var restored = false;
          try {
            var snap =
              item._frozen_fields && typeof item._frozen_fields === "object"
                ? deepCloneJson(item._frozen_fields)
                : null;
            if (snap && typeof snap === "object") {
              if (!item.payload || typeof item.payload !== "object") item.payload = {};
              item.payload.fields = snap;
              payload = item.payload;
              item.payload_hash = payloadHashOf(payload);
              item.material_hash = item.payload_hash;
              restored = stableStringify(snap) === item._frozen_payload_json;
            }
          } catch (eRest) {
            restored = false;
          }
          try {
            if (global.GROps && GROps.report) {
              // Restored shared-ref churn → info-level noise control; true unrestorable → warn.
              GROps.report(
                restored ? "capture_mutation_restored" : "capture_mutated_blocked",
                "upload",
                { batch_id: item.batch_id, restored: !!restored },
                restored ? "info" : "warn"
              );
            }
          } catch (eM) {}
        }
      }
    } catch (ePv) {}
    var planEpoch = null;
    try {
      if (item.plan_epoch != null) planEpoch = Number(item.plan_epoch);
      else if (global.__GR_PLAN_EPOCH__ != null) planEpoch = Number(global.__GR_PLAN_EPOCH__);
      else if (global.GRSessionScheduler && GRSessionScheduler.getPlanEpoch) {
        planEpoch = Number(GRSessionScheduler.getPlanEpoch());
      }
      if (!isFinite(planEpoch) || planEpoch <= 0) planEpoch = null;
    } catch (ePe) {
      planEpoch = null;
    }
    var body = {
      session_id: item.session_id || cfg.session_id || global.__GR_SESSION_ID__ || "",
      batch_id: item.batch_id,
      source: item.source || "main",
      analyze: false,
      inject_path: item.inject_path || cfg.inject_path,
      product_version: productVer || undefined,
      payload: payload,
      capture_id: item.capture_id || undefined,
      material_generation: item.material_generation != null ? item.material_generation : undefined,
      payload_hash: item.payload_hash || undefined,
      material_hash: item.material_hash || item.payload_hash || undefined,
      // iss/72: stamp plan_epoch so server can reject stale route material.
      plan_epoch: planEpoch != null ? planEpoch : undefined,
      // attempt_id is transport-only — changes every send, not material identity.
      attempt_id:
        "att_" +
        String(item._transport_attempts || 0) +
        "_" +
        String(Date.now()),
      source_kind: "fe",
      realm_kind: (function () {
        try {
          var src = String(item.source || "main").toLowerCase();
          if (src.indexOf("worker") >= 0) return "worker";
          if (src.indexOf("sandbox") >= 0) return "sandbox_iframe";
          if (src.indexOf("iframe") >= 0) return "iframe";
          return "document";
        } catch (eRk) {
          return "document";
        }
      })(),
      probe_method_id:
        (item.probe_method_id ||
          item.method_id ||
          (item.method && item.method.method_id) ||
          undefined) || undefined,
    };
    // Never beacon after halt — pagehide would re-flood completed cycles.
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var sameOrigin = false;
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    var ctrl = null;
    try {
      if (typeof AbortController !== "undefined") {
        ctrl = new AbortController();
        try {
          ctrl.__gr_batch_id = String((item && item.batch_id) || "");
        } catch (eTag) {}
        inflightCtrls.push(ctrl);
      }
    } catch (eC) {}
    var fp = sameOrigin || isFirstPartyMode();
    // Gecko/Edge: hard anchors must prefer first-party same-origin (CORS dual-fire 5xx).
    var hard = isHardAnchorBatch(item.batch_id);
    var engineGecko = false;
    var engineEdge = false;
    try {
      var uaE = String((global.navigator && navigator.userAgent) || "");
      // No InstallTrigger (deprecated — typeof alone warns in Firefox).
      try {
        engineGecko =
          typeof global.mozInnerScreenX === "number" ||
          /Firefox\//.test(uaE) ||
          /FxiOS\//.test(uaE) ||
          (/Gecko\//.test(uaE) && !/like Gecko/.test(uaE));
      } catch (eG) {
        engineGecko = /Firefox\//.test(uaE);
      }
      engineEdge = /Edg\//.test(uaE);
    } catch (eEg) {}
    // Gecko/Edge: always first-party same-origin ingest (CORS dual-fire → NetworkError / mid incomplete).
    var forceFpEngine = engineGecko || engineEdge;
    if (forceFpEngine && !sameOrigin) {
      try {
        var p = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
        url = p + "/v1/ingest";
        sameOrigin = true;
        fp = true;
      } catch (eFp) {}
    }

    // Session seal: async prepare (grant from open). Beacon cannot await crypto — skip when sealing.
    // CRITICAL under REQUIRE_SEALED: never plain-fallback (server 426 storm). Lab-only plain when not required.
    function sealNeedNow() {
      try {
        if (global.__GR_FORCE_PLAIN_INGEST__) return false;
        if (global.__GR_SEEN_SEALED_REQUIRED__ || global.__GR_REQUIRE_SEALED__) return true;
        if (global.GRSeal && typeof GRSeal.isRequireSealed === "function" && GRSeal.isRequireSealed())
          return true;
        var b = global.__GR_BOOT__ || {};
        if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) return true;
        // Prod first-party: assume sealed before open/bootstrap lands (kill plain→426 race).
        if (
          (global.__GR_FIRST_PARTY__ || b.first_party || b.firstParty) &&
          (b.inject_path === "nginx" ||
            b.injectPath === "nginx" ||
            (b.env_id && String(b.env_id).indexOf("prod") === 0))
        ) {
          return true;
        }
      } catch (eN) {}
      return false;
    }
    /** Wait for open.seal_grant (or timeout) when sealed required — no plain attempt. */
    function waitOpenSealGrant(timeoutMs) {
      timeoutMs = timeoutMs == null ? 16000 : timeoutMs;
      try {
        if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
          return Promise.resolve(true);
        }
        if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
          if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
          return Promise.resolve(true);
        }
      } catch (e0) {}
      if (global.__GR_SEAL_READY_P__) {
        return Promise.race([
          global.__GR_SEAL_READY_P__.then(function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            return !!(
              (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) ||
              (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64)
            );
          }),
          new Promise(function (resolve) {
            setTimeout(function () {
              resolve(false);
            }, timeoutMs);
          }),
        ]);
      }
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          try {
            if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
              return resolve(true);
            }
            if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
              if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
              return resolve(true);
            }
          } catch (e1) {}
          if (Date.now() - start > timeoutMs) return resolve(false);
          setTimeout(tick, 35);
        }
        tick();
      });
    }

    function ensureSealModule() {
      if (global.GRSeal && typeof GRSeal.prepareUpload === "function") {
        return Promise.resolve(true);
      }
      if (!sealNeedNow()) return Promise.resolve(false);
      if (global.__GR_SEAL_LOAD_P__) return global.__GR_SEAL_LOAD_P__;
      global.__GR_SEAL_LOAD_P__ = new Promise(function (resolve) {
        try {
          var basePath = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
          var ver =
            (global.__GR_SERVER_PRODUCT_VERSION__ ||
              global.__GR_PRODUCT_VERSION__ ||
              (global.__GR_BOOT__ && (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
              "") + "";
          var src =
            basePath +
            "/dist/gr.seal.min.js" +
            (ver ? "?v=" + encodeURIComponent(ver) : "");
          var s = document.createElement("script");
          s.async = true;
          s.src = src;
          s.onload = function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            resolve(!!(global.GRSeal && GRSeal.prepareUpload));
          };
          s.onerror = function () {
            resolve(false);
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (eL) {
          resolve(false);
        }
      });
      return global.__GR_SEAL_LOAD_P__;
    }
    // Seal circuit: stop seal storms without re-collect thrash (GPI Phase C).
  function sealCircuitOpen() {
    try {
      if (global.__GR_SEAL_WASM_DEAD__) return true;
      var c = global.__GR_SEAL_CIRCUIT__;
      if (c && c.open && c.until && Date.now() < c.until) return true;
      if (c && c.open && c.until && Date.now() >= c.until) {
        global.__GR_SEAL_CIRCUIT__ = { open: false, fails: 0, until: 0 };
      }
    } catch (e) {}
    return false;
  }
  function sealCircuitTrip(why) {
    try {
      var c = global.__GR_SEAL_CIRCUIT__ || { fails: 0 };
      c.fails = (c.fails || 0) + 1;
      if (c.fails >= 3) {
        c.open = true;
        c.until = Date.now() + 30000;
        c.why = String(why || "seal_fail");
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_circuit_open",
              "upload",
              { fails: c.fails, until: c.until, why: c.why },
              "warn"
            );
          }
        } catch (eO) {}
      }
      global.__GR_SEAL_CIRCUIT__ = c;
    } catch (e2) {}
  }

  var sealPrep = ensureSealModule()
      .then(function () {
        if (sealCircuitOpen()) {
          var ec = new Error("seal_circuit_open");
          ec.code = "seal_circuit_open";
          ec.seal_failed = true;
          ec.seal_circuit = true;
          throw ec;
        }
        // When sealed required: never race plain — wait open grant first (hard batches especially).
        if (!sealNeedNow()) return true;
        var waitMs = hard || isDeepenBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 20000 : 14000;
        return waitOpenSealGrant(waitMs).then(function (ok) {
          if (ok) return true;
          // Open lag / wiped grant: remint via session open once, then short wait.
          return refreshSealGrant(sessionOf(item)).then(function (got) {
            if (!got) return false;
            return waitOpenSealGrant(4000);
          });
        });
      })
      .then(function () {
      if (!(global.GRSeal && typeof GRSeal.prepareUpload === "function")) {
        if (sealNeedNow()) {
          var eMiss = new Error("seal_module_missing");
          eMiss.code = "seal_failed";
          eMiss.seal_failed = true;
          throw eMiss;
        }
        return {
          url: url,
          body: JSON.stringify(body),
          sealed: false,
        };
      }
      try {
        if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eAd0) {}
      // Prefer authoritative session id after open supersede.
      try {
        var authSid =
          (global.__GR_SESSION_ID__ || cfg.session_id || body.session_id || "") + "";
        if (authSid && body.session_id && authSid !== body.session_id) {
          // Only rewrite when auth looks like a real cycle and grant matches it.
          if (
            GRSeal.grantValid &&
            GRSeal.grantValid(authSid) &&
            String(authSid).indexOf("cycle_") === 0
          ) {
            body.session_id = authSid;
            item.session_id = authSid;
          }
        }
      } catch (eSid) {}
      return GRSeal.prepareUpload(base, body).catch(function (eSeal) {
        var need = sealNeedNow();
        if (need) {
          var es = eSeal || new Error("seal_failed");
          es.code = es.code || "seal_failed";
          es.seal_failed = true;
          // Permanent WASM death (unsupported / not binary / load fail): stop seal retry storm.
          var msgSeal = String(es.message || es || "");
          var wasmDead =
            !!global.__GR_SEAL_WASM_DEAD__ ||
            /seal_wasm_unsupported|wasm_not_binary|wasm_http_|seal_wasm_required/i.test(msgSeal);
          if (wasmDead) {
            try {
              global.__GR_SEAL_WASM_DEAD__ = 1;
              es.code = /unsupported/i.test(msgSeal) ? "seal_wasm_unsupported" : "seal_failed";
              es.terminal = false; // still allow remint if grant path heals; budget handled below
              es.seal_wasm_dead = true;
            } catch (eDead) {}
          }
          try {
            // Cap ops: crawlers emitted 60–70× upload_5xx/session on seal_wasm_required.
            global.__GR_SEAL_OPS_N__ = (global.__GR_SEAL_OPS_N__ || 0) + 1;
            if (global.__GR_SEAL_OPS_N__ <= 3 && global.GROps && GROps.uploadHttp) {
              GROps.uploadHttp(0, item && item.batch_id, {
                seal_failed: true,
                seal_wasm_dead: !!wasmDead,
                code: es.code,
                err: msgSeal.slice(0, 160),
                final: !!wasmDead && global.__GR_SEAL_OPS_N__ >= 2,
              });
            }
          } catch (eOpsS) {}
          throw es;
        }
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_fallback_plain",
              "upload",
              {
                batch: String((item && item.batch_id) || "").slice(0, 48),
                err: String((eSeal && eSeal.message) || eSeal || "").slice(0, 80),
              },
              "warn"
            );
          }
        } catch (eRep) {}
        return {
          url: base + "/v1/ingest",
          body: JSON.stringify(body),
          sealed: false,
        };
      });
    });

    return sealPrep.then(function (wire) {
    url = wire.url || url;
    var bodyStr = wire.body || JSON.stringify(body);
    var sealed = !!wire.sealed;
    // Keep item session aligned with sealed envelope (supersede path).
    try {
      if (wire.session_id && item) item.session_id = wire.session_id;
    } catch (eWs) {}
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo2) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    fp = sameOrigin || isFirstPartyMode();

    // Sealed path: never sendBeacon (async crypto; envelope size). Use fetch+keepalive on pagehide.
    if (hiding && !sealed && global.navigator && navigator.sendBeacon) {
      try {
        var blob = new Blob([bodyStr], { type: "text/plain;charset=UTF-8" });
        if (navigator.sendBeacon(url, blob)) return Promise.resolve({ ok: true, via: "beacon" });
      } catch (e) {}
    }
    // Gecko: avoid keepalive on large hard plain bodies (broken-pipe 5xx).
    // Sealed+hide: always keepalive so unload does not drop encrypted batch (no sendBeacon).
    var useKeepalive = false;
    if (sealed && hiding) {
      useKeepalive = true;
    } else if (hiding || (fp && hard && !engineGecko && !engineEdge)) {
      useKeepalive = true;
    }
    var init = {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: bodyStr,
      // Same-origin: send CF cookies; cross-origin: include when CORS credentials allowed
      credentials: sameOrigin ? "same-origin" : "include",
      mode: sameOrigin ? "same-origin" : "cors",
      keepalive: !!useKeepalive,
    };
    if (ctrl) init.signal = ctrl.signal;
    // Abort hard uploads that hang so SLA can retry.
    // Gecko: hard multipath payloads are large — short abort caused timeout storms
    // (upload_timeout_abort → probe_fail_budget → mid blocked).
    var abortTo = null;
    if (ctrl && (hard || isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id))) {
      try {
        // Heavy deepen needs longer: large sealed body + compress.
        var hardAbortMs = isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id)
          ? engineGecko || engineEdge
            ? 45000
            : 36000
          : engineGecko || engineEdge
            ? 28000
            : 22000;
        abortTo = setTimeout(function () {
          try {
            if (ctrl) ctrl.__gr_timeout_abort = 1;
            ctrl.abort();
          } catch (eAb) {}
        }, hardAbortMs);
      } catch (eT) {}
    }
    // First / high-priority batches: high fetch priority (v57 p50 path)
    try {
      if (sent === 0 || (item.priority || 0) >= 90 || hard) {
        init.priority = "high";
      } else if (fp) {
        // Remaining packs: still elevated so relay hop is not starved by page assets
        init.priority = "high";
      }
    } catch (eP) {}
    return fetch(url, init)
      .finally(function () {
        if (abortTo) {
          try {
            clearTimeout(abortTo);
          } catch (eC2) {}
        }
      })
      .then(function (r) {
        // Drop controller from list when response arrives
        if (ctrl) {
          var ix = inflightCtrls.indexOf(ctrl);
          if (ix >= 0) inflightCtrls.splice(ix, 1);
        }
        // If we halted while request was in flight, do not parse/retry as error flood.
        if (uploadsBlocked(item) && r.status === 410) {
          applyHalt("session_expired", sessionOf(item), "cycle_complete");
          var errH = new Error("upload_halted");
          errH.terminal = true;
          errH.status = 410;
          errH.silent = true;
          errH.code = "cycle_complete";
          throw errH;
        }
        return r
          .json()
          .catch(function () {
            // HTTP 2xx with empty/non-JSON body is still a successful transport —
            // do NOT invent ok:false (was flooding upload_biz_reject with "HTTP 200").
            if (r.ok) {
              return { ok: true, empty_body: true, http: r.status };
            }
            return { ok: false, error: "HTTP " + r.status };
          })
          .then(function (j) {
            var term = parseTerminal(r.status, j || {});
            if (term.terminal) {
              // Halt BEFORE throwing so sibling inflight sees blocked flag.
              applyHalt(term.code || "session_expired", sessionOf(item), term.code);
              // Soft 200 close: resolve quietly (no throw → no red XHR / NS_BINDING_ABORTED cascade).
              if (term.soft_close || (r.status === 200 && term.terminal)) {
                try {
                  if (global.GROps && GROps.report) {
                    GROps.report(
                      "upload_soft_cycle_closed",
                      "upload",
                      {
                        code: term.code,
                        batch: String((item && item.batch_id) || "").slice(0, 48),
                      },
                      "info"
                    );
                  }
                } catch (eSoft) {}
                return j || { ok: true, accepted: false, halt_uploads: true };
              }
              var err = new Error((j && j.error) || "HTTP " + r.status);
              err.terminal = true;
              err.status = r.status;
              err.code = term.code;
              err.body = j;
              err.silent = r.status === 410 || r.status === 200;
              throw err;
            }
            // Transport OK: never treat as biz reject (even if body omitted ok).
            if (r.ok) {
              if (j && j.ok === false && j.error) {
                // Explicit server business reject on 200 — report warn, still accept storage if ingest landed.
                try {
                  if (global.GROps && GROps.uploadHttp) {
                    GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                      ok: false,
                      err: j.error,
                      explicit_biz: true,
                    });
                  }
                } catch (eOpsU0) {}
              }
              // Prior network fail for this batch → recovery event (vtid-tagged).
              try {
                var bidOk = String((item && (item.batch_id || item.pack_id)) || "");
                global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
                var prevFails = global.__GR_NET_FAIL_N__[bidOk] || 0;
                if (prevFails > 0 && global.GROps && GROps.uploadRecovered) {
                  GROps.uploadRecovered(bidOk, {
                    prior_network_fails: prevFails,
                    http: r.status,
                    sealed: !!sealed,
                  });
                  delete global.__GR_NET_FAIL_N__[bidOk];
                }
              } catch (eRec) {}
              return j || { ok: true };
            }
            // HTTP 403/503: sniff CF "I'm Under Attack" / challenge HTML.
            if (r.status === 403 || r.status === 503 || r.status === 429) {
              try {
                var snip =
                  (j && (j.error || j.message || JSON.stringify(j).slice(0, 120))) ||
                  "HTTP " + r.status;
                var snipL = String(snip).toLowerCase();
                var cfHit =
                  /challenge|cf-mitigated|just a moment|attention required|under attack|cloudflare|cf-ray|access denied/i.test(
                    snipL
                  );
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                    ok: false,
                    err: String(snip).slice(0, 100),
                    body_snip: String(snip).slice(0, 100),
                    net_class: cfHit ? "edge_challenge" : "http_" + r.status,
                    cf_challenge: !!cfHit,
                    will_retry: true,
                  });
                }
                var eCf = new Error(String(snip).slice(0, 120));
                eCf.status = r.status;
                eCf.body = j;
                eCf.network = !!cfHit; // retriable when edge challenge
                eCf.code = cfHit ? "upload_edge_challenge" : "upload_http_" + r.status;
                eCf.cf_challenge = !!cfHit;
                throw eCf;
              } catch (eCfThrow) {
                if (eCfThrow && eCfThrow.status) throw eCfThrow;
              }
            }
            if (j && j.ok === false) {
              try {
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(
                    r.status,
                    item && (item.batch_id || item.pack_id),
                    { ok: j && j.ok, err: j && j.error }
                  );
                }
              } catch (eOpsU) {}
              var e2 = new Error((j && j.error) || "HTTP " + r.status);
              e2.status = r.status;
              e2.body = j;
              // Broken pipe / client abort often surfaces as TypeError network fail upstream.
              e2.network = false;
              // 426 sealed_ingest_required: flip require flag, never retry plain.
              try {
                var errTxt = String((j && j.error) || "");
                if (
                  r.status === 426 ||
                  /sealed_ingest_required/i.test(errTxt)
                ) {
                  global.__GR_SEEN_SEALED_REQUIRED__ = true;
                  global.__GR_REQUIRE_SEALED__ = true;
                  global.__GR_BOOT__ = global.__GR_BOOT__ || {};
                  global.__GR_BOOT__.require_sealed_ingest = true;
                  if (global.GRSeal && GRSeal.setRequireSealed) {
                    GRSeal.setRequireSealed(true);
                  }
                  e2.code = "sealed_required";
                  e2.seal_required = true;
                  e2.seal_failed = true; // retriable path (clear sentKeys for hard)
                }
              } catch (e426) {}
              throw e2;
            }
            // Non-JSON error body with 426
            if (r.status === 426) {
              try {
                global.__GR_SEEN_SEALED_REQUIRED__ = true;
                global.__GR_REQUIRE_SEALED__ = true;
                if (global.GRSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
              } catch (e426b) {}
              var e426e = new Error("sealed_ingest_required");
              e426e.status = 426;
              e426e.code = "sealed_required";
              e426e.seal_required = true;
              e426e.seal_failed = true;
              throw e426e;
            }
            return j;
          });
      })
      .catch(function (e) {
        if (ctrl) {
          var iy = inflightCtrls.indexOf(ctrl);
          if (iy >= 0) inflightCtrls.splice(iy, 1);
        }
        // Aborted due to halt / true unload / timeout — silent (never flood ops as network).
        // Tab background (PAGE_BACKGROUNDED) is NOT terminal — keep retrying for maximize probe.
        var msgE = String((e && (e.message || e.name)) || e || "");
        var unloading = isPageUnloading();
        if (
          e &&
          (e.name === "AbortError" ||
            /abort/i.test(msgE) ||
            (haltState && !e.status) ||
            unloading ||
            global.__GR_HALT_UPLOADS__)
        ) {
          var ea = new Error("upload_halted");
          ea.terminal = !!haltState || unloading;
          ea.status = haltState ? 410 : 0;
          ea.silent = true;
          ea.code = (haltState && haltState.code) || (unloading ? "pagehide" : "halt");
          // Soft timeout abort (hard hang): allow retry, not terminal cycle close.
          if (
            !haltState &&
            !unloading &&
            (/abort/i.test(msgE) || (ctrl && ctrl.__gr_timeout_abort))
          ) {
            ea.terminal = false;
            ea.silent = true;
            ea.network = false; // do not flood upload_network
            ea.status = 0;
            ea.code = "upload_timeout_abort";
            ea.timeout_abort = true;
          }
          throw ea;
        }
        // Network / broken-pipe: report once per batch (not every retry).
        // Classify CF challenge / abort / background so ops is not pure "error flood".
        if (e && !e.status) {
          try {
            var bidN = String((item && (item.batch_id || item.pack_id)) || "");
            var hideN = unloading;
            var netKey = "net:" + bidN;
            global.__GR_NET_REPORT__ = global.__GR_NET_REPORT__ || Object.create(null);
            global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
            var lastN = global.__GR_NET_REPORT__[netKey] || 0;
            var nowN = Date.now();
            var msgLow = String(msgE || "").toLowerCase();
            var netClass = "fetch_fail";
            if (e.timeout_abort || /abort/i.test(msgLow)) netClass = "abort";
            else if (
              /challenge|cf-mitigated|just a moment|attention required|access denied|under attack|cf-ray|cloudflare/i.test(
                msgLow
              )
            )
              netClass = "edge_challenge";
            else if (isPageBackgrounded()) netClass = "backgrounded";
            // Do not count silent/timeout aborts as network flood.
            if (e.silent || e.timeout_abort || netClass === "abort") {
              e.network = false;
              e.status = 0;
              e.code = e.code || "upload_timeout_abort";
            } else {
              global.__GR_NET_FAIL_N__[bidN] = (global.__GR_NET_FAIL_N__[bidN] || 0) + 1;
              var attN = global.__GR_NET_FAIL_N__[bidN];
              var maxAtt = maxAttemptsForItem(item);
              var willRetryN = attN < maxAtt && !hideN;
              if (
                !hideN &&
                nowN - lastN > 12000 &&
                global.GROps &&
                GROps.uploadHttp
              ) {
                global.__GR_NET_REPORT__[netKey] = nowN;
                // Sub-classify http:0 Failed to fetch (not server HTTP status).
                // online=false → offline; else client_fetch_fail (conn limit / DNS / TLS / CORS).
                var online =
                  typeof navigator !== "undefined" && navigator.onLine != null
                    ? !!navigator.onLine
                    : null;
                if (online === false) netClass = "offline";
                else if (netClass === "fetch_fail") netClass = "client_fetch_fail";
                var apiBaseN = "";
                var apiHostN = "";
                try {
                  apiBaseN = String(
                    cfg.apiBase ||
                      (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) ||
                      global.__GR_API_BASE__ ||
                      "/g5"
                  );
                  apiHostN =
                    apiBaseN.indexOf("http") === 0
                      ? apiBaseN.split("/").slice(0, 3).join("/")
                      : location && location.host
                        ? String(location.protocol) + "//" + location.host + apiBaseN
                        : apiBaseN;
                } catch (eAb) {}
                // Remember last net_class per batch for exhaust reports.
                try {
                  global.__GR_NET_CLASS_LAST__ =
                    global.__GR_NET_CLASS_LAST__ || Object.create(null);
                  global.__GR_NET_CLASS_LAST__[bidN] = {
                    net_class: netClass,
                    transport: "no_http_response",
                    ts: nowN,
                  };
                } catch (eMem) {}
                GROps.uploadHttp(0, bidN, {
                  network: true,
                  err: msgE.slice(0, 80),
                  net_class: netClass,
                  // Not an HTTP response from API — browser never got status line.
                  transport: "no_http_response",
                  fail_class_hint:
                    netClass === "edge_challenge"
                      ? "edge_challenge"
                      : netClass === "offline"
                        ? "client_offline"
                        : netClass === "abort" || netClass === "backgrounded"
                          ? "client_abort"
                          : "network_env",
                  impact_band_hint: isHeavyBatch(bidN) ? "deepen_optional" : "ops_attention",
                  api_base: apiBaseN.slice(0, 80),
                  api_host: String(apiHostN).slice(0, 120),
                  online: online,
                  backgrounded: isPageBackgrounded(),
                  unloading: !!hideN || isPageUnloading(),
                  pagehide: !!hideN,
                  short_visit: !!hideN || isPageUnloading(),
                  commercial_hard: isCommercialHardForOps(bidN),
                  commercial_land_fast: isCommercialLandFast(bidN),
                  attempt: attN,
                  max_attempts: maxAtt,
                  will_retry: willRetryN,
                  final: !willRetryN,
                  heavy: isHeavyBatch(bidN),
                });
              }
              e.network = true;
              e.status = 0;
              e.code = "upload_network";
            }
          } catch (eNet) {}
        }
        throw e;
      });
    });
  }

  function maxAttempts() {
    return hiding ? cfg.max_attempts_hide : cfg.max_attempts_alive;
  }

  function isRpaBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isRpaBatch) {
        return !!GRProbeLifecycle.isRpaBatch(batchId);
      }
    } catch (eR) {}
    var id = String(batchId || "");
    return id === "B11_interaction" || id.indexOf("B11_") === 0;
  }

  function maxAttemptsForItem(item) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.maxAttemptsFor) {
        var cap = GRProbeLifecycle.maxAttemptsFor(item && item.batch_id);
        if (hiding) return Math.min(cap, cfg.max_attempts_hide || 4);
        // RPA continuous: hard cap 3 alive, 2 on hide — never storm soft_exhausted.
        if (isRpaBatch(item && item.batch_id)) {
          return Math.min(cap, hiding ? 2 : 3);
        }
        return cap;
      }
    } catch (eM) {}
    if (isRpaBatch(item && item.batch_id)) return hiding ? 2 : 3;
    return maxAttempts();
  }

  /**
   * B10 + Lane-C must-land: beat short-visit / pagehide window.
   * Must NOT inherit deepen/heavy 4s+ stretch (lifecycle marks these heavy).
   */
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

  function backoffMsForItem(item, attempt) {
    var a = Math.max(0, (attempt || 1) - 1);
    // Commercial land path: front-load retries inside ~2–4s dwell + keepalive.
    if (isCommercialLandFast(item && item.batch_id)) {
      // 120, 360, 680, 1080, 1560… cap 2000
      return Math.min(2000, 120 + a * 240 + a * a * 40);
    }
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.backoffMsFor) {
        return GRProbeLifecycle.backoffMsFor(item && item.batch_id, attempt);
      }
    } catch (eB) {}
    return Math.min(20000, 1000 * Math.pow(2, a));
  }

  function scheduleRetry(item, attempt) {
    var delay = backoffMsForItem(item, attempt);
    // Heavy network fails: longer backoff so browser connection budget recovers.
    // Never stretch commercial land — short visits lose B10/Lane-C otherwise.
    try {
      var bid = item && item.batch_id;
      if (!isCommercialLandFast(bid) && isHeavyBatch(bid)) {
        delay = Math.max(delay, 4000 + Math.min(20000, (attempt || 1) * 3000));
      }
    } catch (eHb) {}
    // iss/70: transport-only retry — keep frozen capture; do not re-collect.
    try {
      freezeCapture(item);
      transportRetryCount++;
      item._transport_attempts = (item._transport_attempts || 0) + 1;
    } catch (eTr) {}
    setTimeout(function () {
      if (haltState && !allowDespiteHalt(item)) return;
      if (uploadsBlocked(item)) return;
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          return;
        }
      } catch (eA) {}
      // Coalesce force retries (same key) — avoid pending storms after transport fail.
      queuePending(item);
      pump();
    }, delay);
  }

  function pump() {
    try {
      pruneStaleInflight();
    } catch (ePr) {}
    var conc = effectiveUploadCap();
    if (conc <= 0) return;
    // After halt, still pump B10x / force_after_halt (EDH must-land).
    if (
      haltState &&
      !pending.some(function (p) {
        return allowDespiteHalt(p);
      })
    ) {
      return;
    }
    // Never start more than conc slots even if counter drifted.
    if (inflight > conc) inflight = Math.min(inflight, inflightItems.length, conc);
    while (inflight < conc && pending.length) {
      pending.sort(function (a, b) {
        return effectivePriority(b) - effectivePriority(a);
      });
      var item = pending.shift();
      if (uploadsBlocked(item)) {
        haltedDrops++;
        continue;
      }
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          // Soft quiet budget: drop soft packs, keep hard.
          if (!isHardAnchorBatch(item.batch_id)) {
            failed++;
            continue;
          }
        }
      } catch (eQ) {}
      // Deferred retry: not ready yet.
      if (item._retry_after_ms && item._retry_after_ms > Date.now()) {
        queuePending(item);
        break;
      }
      // P1: serialize heavy (B10x/B7) — max 1 inflight to cut Failed to fetch storms.
      var itemHeavy = isHeavyBatch(item.batch_id);
      if (itemHeavy && heavyInflight >= heavyMaxInflight()) {
        // Prefer a light pack next; requeue this heavy for later.
        var lightIdx = -1;
        for (var hi = 0; hi < pending.length; hi++) {
          if (!isHeavyBatch(pending[hi].batch_id)) {
            lightIdx = hi;
            break;
          }
        }
        if (lightIdx >= 0) {
          var light = pending.splice(lightIdx, 1)[0];
          queuePending(item);
          item = light;
          itemHeavy = false;
        } else {
          pending.unshift(item);
          break;
        }
      }
      var k = keyOf(item);
      attempts[k] = (attempts[k] || 0) + 1;
      if (hiding && isHardAnchorBatch(item.batch_id) && attempts[k] > 1) {
        hardRetries++;
      }
      inflight++;
      inflightItems.push(item);
      registryUpsertFromItem(item, "uploading");
      if (itemHeavy) heavyInflight++;
      // iss P0: bind each in-flight upload to its own item/k/heavy — never close over
      // function-scoped `var` from the pump while-loop (concurrent ACK mis-attribution).
      startUpload(item, itemHeavy, k);
    }
  }

  function startUpload(item, itemHeavy, k) {
      postOne(item)
        .then(function (resp) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          if (haltState) {
            pump();
            return;
          }
          sent++;
          if (isHardAnchorBatch(item.batch_id)) hardFlushed++;
          // iss/72 P0-4: only verified ACK → sentKeys / batches_ok.
          try {
            var ackRes = applyAck(item, resp || {});
            var ackT = ackRes && typeof ackRes === "object" ? ackRes.type : ackRes;
            var verified = ackRes && typeof ackRes === "object" ? !!ackRes.verified : false;
            if (ackT === "conflict") {
              try {
                if (global.GROps && GROps.report) {
                  GROps.report(
                    "upload_ack_conflict",
                    "upload",
                    {
                      batch_id: item.batch_id,
                      generation: item.material_generation,
                      payload_hash: item.payload_hash,
                    },
                    "warn"
                  );
                }
              } catch (eCf) {}
            } else if (verified) {
              sentKeys[k] = true;
              try {
                sentKeys[k + "|ts"] = Date.now();
              } catch (eTs) {}
              if (resp && resp.sealed === false) noteOutcome("plain_ok", item.batch_id, item);
              else noteOutcome("sealed_ok", item.batch_id, item);
              registryUpsertFromItem(item, "acked");
            } else {
              // transport_ok_unverified — do not short-circuit future retries incorrectly
              registryUpsertFromItem(item, "transport_ok_unverified");
            }
          } catch (eOut) {}
          try {
            aimdOnSuccess();
          } catch (eAim) {}
          // Mild ramp after first success — never exceed concurrency_max.
          if (!ramped && cfg.ramp_after_first) {
            ramped = true;
            var maxC = Number(cfg.concurrency_max) || 6;
            var rampT = Math.min(maxC, Number(cfg.ramp_after_first) || 4);
            if (cfg.concurrency < rampT) cfg.concurrency = rampT;
          }
          // Success-path terminal signals (complete / cool) without waiting for 410.
          var termOk = parseTerminal(200, resp || {});
          if (termOk.terminal) {
            applyHalt(termOk.code || "probe_complete", sessionOf(item), termOk.code);
          } else {
            maybeLocalIdentityDone();
          }
          try {
            // Keep client-side received inventory for hard-SLA (no cache-clear required).
            global.__GR_RECEIVED_BATCHES__ = global.__GR_RECEIVED_BATCHES__ || [];
            global.__GR_RECEIVED_BATCHES__.push({
              batch_id: item.batch_id,
              source: item.source || "main",
            });
            global.dispatchEvent(
              new CustomEvent("gr-upload-ok", {
                detail: {
                  batch_id: item.batch_id,
                  source: item.source || "main",
                  response: resp || null,
                  cycle_probe_status: (resp && resp.cycle_probe_status) || null,
                },
              })
            );
          } catch (eEv) {}
          pump();
        })
        .catch(function (err) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          try {
            if (err && (err.seal_required || err.status === 426)) noteOutcome("sealed_reject", item.batch_id);
            else if (err && err.network) noteOutcome("network", item.batch_id);
            else if (err && !err.silent) noteOutcome("other_fail", item.batch_id);
          } catch (eOut2) {}
          try {
            if (!err || !err.silent) aimdOnFail();
          } catch (eAimF) {}
          try {
            registryUpsertFromItem(item, "retry_wait");
          } catch (eReg) {}
          // Terminal: cycle done / cool / session_expired — never retry flood.
          if (err && err.terminal) {
            applyHalt(
              err.code || "session_expired",
              sessionOf(item),
              err.code || "session_expired"
            );
            if (!err.silent) failed++;
            else haltedDrops++;
            try {
              if (!err.silent) {
                global.dispatchEvent(
                  new CustomEvent("gr-upload-terminal", {
                    detail: {
                      batch_id: item.batch_id,
                      status: err.status,
                      code: err.code,
                      error: String(err.message || err),
                    },
                  })
                );
              }
            } catch (eT) {}
            // Do not pump after terminal — queue drained in applyHalt.
            return;
          }
          if (uploadsBlocked(item) || haltState) {
            haltedDrops++;
            return;
          }
          try {
            if (!err || !err.silent) {
              global.dispatchEvent(
                new CustomEvent("gr-upload-fail", {
                  detail: {
                    batch_id: item.batch_id,
                    pack_id: item.pack_id || item.batch_id,
                    status: err && err.status,
                    code: err && err.code,
                    network: !!(err && err.network),
                    error: String((err && err.message) || err || ""),
                  },
                })
              );
            }
          } catch (eFailEv) {}
          // Hard / deepen / seal fail: clear sentKeys so re-upload can land (never stick "sent").
          try {
            var bidR = String(item.batch_id || "");
            var retriableFail =
              (err && err.network) ||
              (err && err.cf_challenge) ||
              (err &&
                (err.code === "seal_failed" ||
                  err.code === "sealed_required" ||
                  err.seal_failed ||
                  err.seal_required)) ||
              (err && err.timeout_abort) ||
              (err && err.status === 426) ||
              // Edge 403/503 (CF under-attack / rate) — retry hard/deepen anchors.
              (err &&
                (err.status === 403 || err.status === 503 || err.status === 429) &&
                (isHardAnchorBatch(bidR) || isDeepenBatch(bidR)));
            if (retriableFail) {
              // Always clear dedupe on seal/426 so sealed retry can land any batch.
              if (
                err.seal_failed ||
                err.seal_required ||
                err.status === 426 ||
                isHardAnchorBatch(bidR) ||
                isDeepenBatch(bidR) ||
                bidR === "B27_storage_privacy" ||
                bidR === "B65_client_hints_full" ||
                bidR === "B18_webgpu" ||
                bidR === "B46_audio_deep" ||
                bidR === "B55_webrtc_ice_deep" ||
                bidR === "B2_hardware" ||
                bidR === "B3_system" ||
                bidR === "B0_bootstrap" ||
                bidR === "B1_conflict" ||
                bidR === "B11_interaction" ||
                bidR === "B12_anti_camouflage" ||
                bidR === "B7_sandbox"
              ) {
                delete sentKeys[k];
              }
            }
          } catch (eSk) {}
          // seal_failed / 426 is never cycle-terminal — always retry path above.
          var isSealRace =
            err &&
            (err.code === "seal_failed" ||
              err.code === "sealed_required" ||
              err.seal_failed ||
              err.seal_required ||
              err.status === 426 ||
              /seal_grant|seal_timeout|seal_module/i.test(String((err && err.message) || "")));
          if (isSealRace) {
            try {
              err.terminal = false;
            } catch (eTerm) {}
            try {
              sealCircuitTrip(err && (err.code || err.message));
            } catch (eCt) {}
            // Permanent WASM failure: limited seal waits then stop (no 70× storm).
            try {
              var wasmDeadQ =
                !!(err && err.seal_wasm_dead) ||
                !!global.__GR_SEAL_WASM_DEAD__ ||
                /seal_wasm_unsupported|wasm_not_binary|seal_wasm_required/i.test(
                  String((err && err.message) || "")
                );
              item._seal_waits = (item._seal_waits || 0) + 1;
              if (wasmDeadQ && item._seal_waits >= 2) {
                err.terminal = true;
                err.code = err.code || "seal_wasm_dead";
                // Do not decrement attempt — count as real fail.
              } else if ((attempts[k] || 0) > 0) {
                // Do not burn hard-attempt budget on open-lag seal races.
                attempts[k] = Math.max(0, attempts[k] - 1);
              }
              // Circuit open: defer retries longer (transport-only, no re-collect).
              if (sealCircuitOpen()) {
                item._retry_after_ms = Date.now() + 8000;
              }
            } catch (eDec) {}
            // Align session with grant before retry (open supersede).
            try {
              var fixSid =
                (global.__GR_SESSION_ID__ ||
                  (global.GRSeal &&
                    global.GRSeal.resolveSealSessionId &&
                    GRSeal.resolveSealSessionId(item.session_id)) ||
                  cfg.session_id ||
                  "") + "";
              if (fixSid && String(fixSid).indexOf("cycle_") === 0) {
                item.session_id = fixSid;
              }
            } catch (eFix) {}
            // Remint grant in parallel (coalesced).
            try {
              refreshSealGrant(sessionOf(item));
            } catch (eRf) {}
          }
          var cap = maxAttemptsForItem(item);
          var nTry = attempts[k] || 1;
          var sealWaits = item._seal_waits || 0;
          // Seal race: large seal_wait budget; real attempts still capped separately.
          if (isSealRace) {
            cap = Math.max(cap, isHardAnchorBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 14 : 10);
            // Allow continue while seal waits < 16 even if attempt counter looks high.
            if (sealWaits < 16 && nTry >= cap) {
              nTry = cap - 1;
              attempts[k] = nTry;
            }
          }
          if (nTry < cap || (isSealRace && sealWaits < 16)) {
            if (isHardAnchorBatch(item.batch_id) && !isSealRace) hardRetries++;
            if ((item.priority || 0) >= 70) {
              item.priority = Math.min(100, (item.priority || 70) + 1);
            }
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail && !isSealRace) {
                GRProbeLifecycle.recordFail(
                  item.batch_id,
                  (err && (err.code || (err.timeout_abort && "upload_timeout_abort") || err.message)) ||
                    "upload_fail"
                );
              }
            } catch (eLf) {}
            // Seal grant race: wait for remint / open grant (not exponential storm).
            if (isSealRace) {
              var sealDelay = Math.min(4000, 300 + sealWaits * 400);
              item._retry_after_ms = Date.now() + sealDelay;
              queuePending(item);
              setTimeout(function () {
                try {
                  if (item._retry_after_ms && item._retry_after_ms <= Date.now()) {
                    delete item._retry_after_ms;
                  }
                } catch (eClr) {}
                pump();
              }, sealDelay + 20);
              pump();
            } else {
              // Delayed requeue — avoid spin retry storms.
              scheduleRetry(item, nTry);
              pump();
            }
          } else {
            failed++;
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail) {
                var fr = GRProbeLifecycle.recordFail(item.batch_id, "attempts_exhausted");
                // Dedupe exhausted reports per batch (was flooding ops).
                var exKey = "ex:" + String(item.batch_id || "");
                global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                var lastEx = global.__GR_EXHAUST_REPORT__[exKey] || 0;
                var nowEx = Date.now();
                if (nowEx - lastEx > 30000 && global.GROps && GROps.report) {
                  global.__GR_EXHAUST_REPORT__[exKey] = nowEx;
                  // Commercial hard only → error; B10x/B47 network exhaust → warn (v150 fix).
                  var commercialHard = isCommercialHardForOps(item.batch_id);
                  var deepenish =
                    isDeepenBatch(item.batch_id) ||
                    isHeavyBatch(item.batch_id) && !commercialHard;
                  var exCode = commercialHard
                    ? "upload_hard_exhausted"
                    : deepenish
                      ? "upload_deepen_exhausted"
                      : "upload_soft_exhausted";
                  var exSev = commercialHard ? "error" : "warn";
                  // Partial success: B10 already landed + only deepen failed → always warn.
                  try {
                    if (
                      !commercialHard &&
                      sessionOutcome.batches_ok &&
                      sessionOutcome.batches_ok["B10_hw_curves"]
                    ) {
                      exSev = "warn";
                    }
                  } catch (ePart) {}
                  var apiHost = "";
                  try {
                    var ab = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5");
                    apiHost = ab.indexOf("http") === 0 ? ab.split("/").slice(0, 3).join("/") : ab;
                  } catch (eH) {}
                  // Ops dimensions: attach last net_class for this batch (if any).
                  var lastNc = null;
                  var lastTransport = null;
                  try {
                    var netHist = global.__GR_NET_CLASS_LAST__ || Object.create(null);
                    var nk = String(item.batch_id || "");
                    if (netHist[nk]) {
                      lastNc = netHist[nk].net_class || null;
                      lastTransport = netHist[nk].transport || null;
                    }
                  } catch (eNc) {}
                  GROps.report(
                    exCode,
                    "upload",
                    {
                      batch_id: item.batch_id,
                      attempts: nTry,
                      over_budget: !!(fr && fr.overBudget),
                      commercial_hard: commercialHard,
                      deepen: !!deepenish,
                      b10_already_ok: !!(
                        sessionOutcome.batches_ok &&
                        sessionOutcome.batches_ok["B10_hw_curves"]
                      ),
                      api_base: apiHost,
                      online:
                        typeof navigator !== "undefined" && navigator.onLine != null
                          ? !!navigator.onLine
                          : null,
                      // taxonomy dimensions (server also normalizes)
                      net_class: lastNc || undefined,
                      transport: lastTransport || undefined,
                      fail_class_hint: commercialHard
                        ? "hard_exhaust"
                        : deepenish
                          ? "deepen_exhaust"
                          : "soft_exhaust",
                      impact_band_hint: commercialHard
                        ? "commercial_critical"
                        : "deepen_optional",
                    },
                    exSev
                  );
                }
              }
            } catch (eEx) {}
            pump();
          }
        });
  }

  var api = {
    configure: function (o) {
      o = o || {};
      var prevSid = cfg.session_id;
      Object.keys(o).forEach(function (k) {
        if (o[k] !== undefined) cfg[k] = o[k];
      });
      // New session id → isolate queue/registry (iss/72 P1-2).
      if (o.session_id && String(o.session_id) !== String(prevSid || "")) {
        if (haltState && haltState.session_id !== String(o.session_id)) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
          } catch (eC) {}
        }
        resetSessionState("configure_session_id");
      }
      // First-party /g5: raise caps only within min/max budget.
      applyFirstPartyPerfHints();
    },
    /** Mark keys already retained server-side (from open/session received list). */
    seedSentKeys: function (keys) {
      (keys || []).forEach(function (k) {
        if (typeof k === "string") sentKeys[k] = true;
        else if (k && k.batch_id) {
          sentKeys[keyOf(k)] = true;
        }
      });
    },
    /**
     * Clear dedupe keys so force_recollect can re-upload the same batch
     * (e.g. B2 present without residual → brain requests recollect).
     */
    clearSentKeys: function (items) {
      (items || []).forEach(function (it) {
        if (typeof it === "string") {
          delete sentKeys[it];
        } else if (it && it.batch_id) {
          var bidClr = String(it.batch_id);
          // Budget: after terminal ok, only one force_recollect clear per session.
          if (sessionOutcome.batches_ok[bidClr] && forceRecollectUsed[bidClr]) {
            return;
          }
          if (sessionOutcome.batches_ok[bidClr]) {
            forceRecollectUsed[bidClr] = 1;
          }
          delete sentKeys[keyOf(it)];
          try {
            delete sessionOutcome.batches_ok[bidClr];
            delete sessionOutcome.batches_started[bidClr];
          } catch (eOk) {}
        }
      });
    },
    /**
     * Whether brain force_recollect may clear + re-upload this batch.
     * Terminal ok batches: at most once per session.
     */
    allowForceRecollect: function (batchId) {
      var bid = String(batchId || "");
      if (!bid) return false;
      if (!sessionOutcome.batches_ok[bid]) return true;
      if (forceRecollectUsed[bid]) return false;
      forceRecollectUsed[bid] = 1;
      return true;
    },
    enqueue: function (item) {
      if (!item || !item.batch_id) return;
      var bidEnq = String(item.batch_id || "");
      var recollect =
        item.force_recollect === true ||
        item.forceRecollect === true ||
        item.reset_attempts === true;
      // New material generation on explicit recollect.
      if (recollect) {
        try {
          item._capture_frozen = false;
          item.material_generation = (item.material_generation || 1) + 1;
          item.capture_id = null;
        } catch (eRc) {}
      }
      // Success short-circuit: already sealed_ok → skip unless brain force_recollect.
      // (Was: B10x/secondary always force=true → 5–10× wire storms, iss/65.)
      if (batchAlreadyOk(bidEnq) && !recollect) {
        // Explicit force without recollect still blocked once ok (start heartbeat spam).
        successShortCircuit++;
        skipped++;
        return;
      }
      // pagehide: reject non-allowlisted packs immediately.
      if ((hiding || isPageUnloading()) && !isPagehideAllowlisted(bidEnq) && !recollect) {
        pagehideDropped++;
        skipped++;
        return;
      }
      // B10x EDH: force only until first sealed_ok (or recollect). Survive halt/cool.
      if (isB10xBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Secondary silicon/infra: same — once landed, no force storm.
      if (isSecondaryInfraBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Hard/deepen attempt ceiling across SLA re-kicks (prevents attempts:16 storms).
      try {
        var kCap = keyOf(item);
        var capEnq = maxAttemptsForItem(item);
        // B10 / B10x / seal remint: higher ceiling + grant-ready reset (short-visit residual).
        if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if (item.reset_attempts) {
          delete attempts[kCap];
          delete item._seal_waits;
        }
        // If grant is now valid after seal race, clear fake exhaustion and continue.
        try {
          if (
            (attempts[kCap] || 0) >= capEnq &&
            global.GRSeal &&
            GRSeal.grantRawValid &&
            GRSeal.grantRawValid()
          ) {
            attempts[kCap] = Math.max(0, capEnq - 4);
            item._seal_waits = 0;
          }
        } catch (eGr) {}
        // Secondary infra always gets elevated attempt cap
        if (isSecondaryInfraBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if ((attempts[kCap] || 0) >= capEnq && !item.reset_attempts) {
          // Soft re-arm hard/B10x every 8s instead of permanent drop.
          if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id) || isSecondaryInfraBatch(item.batch_id)) {
            var lastCap = item._cap_block_ms || 0;
            var rearmMs = isHardAnchorBatch(item.batch_id) && !isB10xBatch(item.batch_id) ? 6000 : 8000;
            if (Date.now() - lastCap > rearmMs) {
              item._cap_block_ms = Date.now();
              attempts[kCap] = Math.max(0, capEnq - 5);
              item._seal_waits = 0;
              try {
                refreshSealGrant(sessionOf(item));
              } catch (eR2) {}
            } else {
              skipped++;
              try {
                if (global.GROps && GROps.report) {
                  var exK = "excap:" + String(item.batch_id || "");
                  global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                  var nowC = Date.now();
                  if (nowC - (global.__GR_EXHAUST_REPORT__[exK] || 0) > 60000) {
                    global.__GR_EXHAUST_REPORT__[exK] = nowC;
                    GROps.report(
                      "upload_hard_exhausted",
                      "upload",
                      {
                        batch_id: item.batch_id,
                        attempts: attempts[kCap],
                        reason: "enqueue_cap",
                        rearm_ms: rearmMs,
                      },
                      "warn"
                    );
                  }
                }
              } catch (eCap) {}
              return;
            }
          } else {
            skipped++;
            return;
          }
        }
      } catch (eEnqCap) {}
      // Hard gate: after 410/complete never enqueue (ignore force — force only skips dedupe),
      // except B10x / force_after_halt.
      if (uploadsBlocked(item) && !allowDespiteHalt(item)) {
        haltedDrops++;
        skipped++;
        return;
      }
      try {
        if (
          (global.__GR_STOP_PROBE__ || global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) &&
          !allowDespiteHalt(item)
        ) {
          haltedDrops++;
          skipped++;
          return;
        }
      } catch (eStop) {}
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          if (!isHardAnchorBatch(item.batch_id)) {
            skipped++;
            return;
          }
        }
      } catch (eAe) {}
      var k = keyOf(item);
      // RPA continuous: never force-bypass queue coalescing (gecko flood).
      var rpa = isRpaBatch(item.batch_id);
      // Terminal success short-circuit (not start heartbeat). force alone is not enough.
      if (batchAlreadyOk(bidEnq) && !recollect) {
        successShortCircuit++;
        skipped++;
        return;
      }
      // sentKeys: allow one upgrade from start→done (started marked sentKeys but not batches_ok).
      if (sentKeys[k] && !item.force && !recollect && batchAlreadyOk(bidEnq)) {
        skipped++;
        return;
      }
      if (sentKeys[k] && !item.force && !recollect && !isStartHeartbeatItem(item)) {
        // Final payload after start: need force or clear. midEnqueue sets force when !alreadySent;
        // after start alreadySent is true — allow if only batches_started (not batches_ok).
        if (!sessionOutcome.batches_started[bidEnq] || sessionOutcome.batches_ok[bidEnq]) {
          skipped++;
          return;
        }
        // Upgrade path: start was sent, final multipath ready — allow once.
        item.force = true;
      }
      // For RPA, even force: coalesce; never on unload (pagehide drops B11).
      if (rpa && (hiding || isPageUnloading())) {
        pagehideDropped++;
        skipped++;
        return;
      }
      if (rpa && sentKeys[k] && !item.pagehide_flush && !recollect) {
        // Allow re-upload only after quiet window (30s) — was 5s and still noisy.
        var lastOk = sentKeys[k + "|ts"] || 0;
        if (Date.now() - lastOk < 30000) {
          skipped++;
          return;
        }
      }
      // Freeze only after all dedupe, lifecycle, pagehide, and terminal gates.
      // Self-heal may replay many descriptors after cycle completion; dropped
      // items must not pay the deep-clone/hash cost or inflate capture metrics.
      try {
        if (item.payload) freezeCapture(item);
      } catch (eFzEnq) {}
      // Dedup pending by session|batch|source — including force (dense hedge / secondary).
      // Previous: force=true skipped replace → same B19 could stack dozens of times.
      queuePending(item);
      pump();
    },
    /**
     * When open returns a different cycle id than the client mint (completed sticky
     * bag superseded), retarget pending items so they land on the live cycle.
     */
    rewriteSessionId: function (fromId, toId) {
      var from = String(fromId || "");
      var to = String(toId || "");
      if (!from || !to || from === to) return 0;
      var n = 0;
      for (var i = 0; i < pending.length; i++) {
        var it = pending[i];
        if (!it) continue;
        var sid = String(it.session_id || cfg.session_id || "");
        if (sid === from || !it.session_id) {
          it.session_id = to;
          n++;
        }
      }
      try {
        cfg.session_id = to;
      } catch (eC) {}
      // Drop sentKeys bound to the dead cycle so re-upload of same batch is allowed.
      try {
        Object.keys(sentKeys).forEach(function (k) {
          if (String(k).indexOf(from + "|") === 0) delete sentKeys[k];
        });
      } catch (eK) {}
      // If we halted only because of 410 on the dead cycle, allow the live cycle to proceed.
      if (haltState && String(haltState.session_id || "") === from) {
        haltState = null;
        try {
          global.__GR_HALT_UPLOADS__ = false;
          global.__GR_STOP_PROBE__ = false;
          global.__GR_SKIP_IDENTITY__ = false;
          global.__GR_CYCLE_CLOSED__ = null;
          global.__GR_UPLOAD_HALT__ = null;
          global.__GR_PHASE__ = "active";
          if (!cfg.concurrency || cfg.concurrency < 1) cfg.concurrency = 3;
          // Undo false local cool stamped by 410 on the superseded bag.
          var S = global.GRStorage;
          if (S && S.setCoolUntil) S.setCoolUntil(0);
        } catch (eR) {}
        pump();
      }
      // Open supersede often arrives with seal_grant for `to` — wake seal waiters.
      pump();
      return n;
    },
    /** Wake pump after open.seal_grant / hot-swap re-eval (B0 may be grant-waiting). */
    kick: function () {
      try {
        if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eK) {}
      // Clear stale seal retry gates so grant-ready items run immediately.
      try {
        for (var i = 0; i < pending.length; i++) {
          if (pending[i] && pending[i]._retry_after_ms) {
            delete pending[i]._retry_after_ms;
          }
        }
      } catch (eR) {}
      pump();
    },
    /** Stop identity uploads for a cycle (cool open / 410 complete). */
    halt: function (reason, sessionId, code) {
      applyHalt(reason || "halt", sessionId, code);
      return haltState;
    },
    isHalted: function (sessionId) {
      if (!haltState) return false;
      if (sessionId == null || sessionId === "") return true;
      return !haltState.session_id || haltState.session_id === String(sessionId);
    },
    haltState: function () {
      return haltState ? Object.assign({}, haltState) : null;
    },
    /** Snapshot of FE local view for multi-party reconcile with BE. */
    clientView: function () {
      var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
      var sent = [];
      try {
        Object.keys(sentKeys).forEach(function (k) {
          var parts = String(k).split("|");
          if (parts.length >= 2) sent.push(parts[1]);
        });
      } catch (eS) {}
      return {
        session_id: sid,
        stop_probe: !!global.__GR_STOP_PROBE__,
        skip_identity: !!global.__GR_SKIP_IDENTITY__,
        halted: !!haltState,
        phase: global.__GR_PHASE__ || null,
        local_uploads_done: !!(
          global.__GR_IDENTITY_UPLOADS_DONE__ &&
          global.__GR_IDENTITY_UPLOADS_DONE__.session_id === sid
        ),
        sent_batch_ids: sent,
        last_http_status: (haltState && haltState.status) || null,
        last_error_code: (haltState && haltState.code) || null,
      };
    },
    /**
     * Apply BE reconcile corrections (server authoritative for cycle lifecycle).
     * Returns applied action names.
     */
    applyCorrections: function (reconcileBody) {
      var applied = [];
      if (!reconcileBody) return applied;
      var sid =
        (reconcileBody.session_id ||
          cfg.session_id ||
          global.__GR_SESSION_ID__ ||
          "") + "";
      try {
        if (reconcileBody.cycle_probe_status) {
          global.__GR_CYCLE_PROBE_STATUS__ = reconcileBody.cycle_probe_status;
        }
        if (reconcileBody.business_state) {
          global.__GR_BUSINESS_STATE__ = reconcileBody.business_state;
        }
      } catch (eG) {}
      var list = reconcileBody.corrections || [];
      for (var i = 0; i < list.length; i++) {
        var c = list[i] || {};
        var act = c.action || "";
        if (act === "fe_halt_uploads" || reconcileBody.fe_should_halt) {
          // Refuse cool-halt while primary residual not acked and still in flight/pending.
          // Server may claim identity_complete_cool from B10x-only has_b10 false positive.
          var refuseHalt = false;
          try {
            var hasPrimary =
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) ||
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]);
            var hardPending = false;
            var pi;
            for (pi = 0; pi < pending.length; pi++) {
              if (isHardAnchorBatch(pending[pi] && pending[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            for (pi = 0; !hardPending && pi < inflightItems.length; pi++) {
              if (isHardAnchorBatch(inflightItems[pi] && inflightItems[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            var reasonStr = String((c && c.reason) || reconcileBody.business_state || "");
            if (
              !hasPrimary &&
              (hardPending || reasonStr.indexOf("identity_complete") >= 0 || reasonStr.indexOf("cycle_status=complete") >= 0)
            ) {
              refuseHalt = true;
            }
          } catch (eRef) {}
          if (refuseHalt) {
            applied.push("fe_halt_uploads_refused_missing_b10");
          } else {
            applyHalt(c.reason || "reconcile_halt", sid, c.reason || "reconcile");
            applied.push("fe_halt_uploads");
          }
        } else if (
          act === "fe_resume_if_same_active_cycle" ||
          act === "fe_clear_false_cool" ||
          reconcileBody.fe_should_resume
        ) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
            global.__GR_CYCLE_CLOSED__ = null;
            global.__GR_STOP_PROBE__ = false;
            global.__GR_SKIP_IDENTITY__ = false;
            global.__GR_PHASE__ = "active";
          } catch (eR) {}
          applied.push(act || "fe_resume");
        } else if (act === "fe_mark_local_done_only") {
          maybeLocalIdentityDone();
          applied.push(act);
        } else if (act === "rebind_session_id") {
          applied.push(act);
        } else if (act === "be_completed_cycle" || act === "be_complete_cycle") {
          applyHalt("be_complete", sid, "cycle_complete");
          applied.push(act);
        }
      }
      if (reconcileBody.fe_should_halt && !haltState) {
        applyHalt("reconcile_halt", sid, reconcileBody.business_state || "halt");
        applied.push("fe_should_halt");
      }
      if (reconcileBody.aligned && reconcileBody.halt_uploads && !haltState) {
        applyHalt("aligned_halt", sid, reconcileBody.business_state || "halt");
        applied.push("aligned_halt");
      }
      try {
        global.dispatchEvent(
          new CustomEvent("gr-status-reconciled", {
            detail: {
              applied: applied,
              body: reconcileBody,
              session_id: sid,
            },
          })
        );
      } catch (eE) {}
      return applied;
    },
    /** New page cycle: clear halt so a fresh session_id can upload. */
    resume: function (newSessionId) {
      if (newSessionId && haltState && haltState.session_id === String(newSessionId)) {
        // Same id still closed.
        return haltState;
      }
      haltState = null;
      // Restore pump capacity for a new cycle.
      if (!cfg.concurrency || cfg.concurrency < 1) {
        cfg.concurrency = 3;
        ramped = false;
      }
      try {
        global.__GR_UPLOAD_HALT__ = null;
        global.__GR_CYCLE_CLOSED__ = null;
        global.__GR_HALT_UPLOADS__ = false;
        // Do not clear STOP_PROBE here — boot decides for cool windows.
      } catch (eR) {}
      return null;
    },
    /** B0 enqueued: allow parallel rest while first upload still inflight. */
    softRamp: function (n) {
      var maxC = Number(cfg.concurrency_max) || 6;
      var target = Math.min(maxC, Number(n) || cfg.mid_ramp_concurrency || 4);
      if (!ramped && cfg.concurrency < target) {
        cfg.concurrency = target;
      }
      pump();
    },
    /**
     * Flush pending uploads.
     * - pagehide / beforeunload / unload → true unload (PAGE_HIDING, lower hide attempts)
     * - hidden / background → tab not focused; keep alive retries; do NOT stop probe
     */
    flush: function (reason) {
      flushReason = reason || "flush";
      var r = String(reason || "");
      var unloading =
        r === "pagehide" || r === "beforeunload" || r === "unload" || r === "close";
      var background = r === "hidden" || r === "background" || r === "visibility_hidden";
      if (unloading) {
        hiding = true;
        try {
          global.__GR_PAGE_HIDING__ = true;
          global.__GR_PAGE_UNLOADING__ = true;
        } catch (e) {}
        // Abort non-allowlisted inflight so keepalive slots go to hard/B10x (iss/65).
        try {
          var keptCtrls = [];
          for (var ai = 0; ai < inflightCtrls.length; ai++) {
            var c = inflightCtrls[ai];
            var cBid = c && c.__gr_batch_id ? String(c.__gr_batch_id) : "";
            if (cBid && isPagehideAllowlisted(cBid)) {
              keptCtrls.push(c);
              continue;
            }
            try {
              if (c && typeof c.abort === "function") {
                c.__gr_soft_unload_abort = 1;
                c.abort();
                softAbortOnUnload++;
              }
            } catch (eAb) {}
          }
          inflightCtrls = keptCtrls;
        } catch (eAc) {}
        // Drop pending non-allowlisted (mid/R/B11) — boost alone is not enough.
        try {
          var kept = [];
          for (var pi = 0; pi < pending.length; pi++) {
            var pit = pending[pi];
            if (!pit) continue;
            if (isPagehideAllowlisted(pit.batch_id) && !batchAlreadyOk(pit.batch_id)) {
              kept.push(pit);
            } else {
              pagehideDropped++;
            }
          }
          pending = kept;
        } catch (eDrop) {}
        // Keepalive window is tiny — max concurrency for remaining hard/B10x only.
        try {
          var maxC = Number(cfg.concurrency_max) || 6;
          cfg.concurrency = Math.max(Number(cfg.concurrency) || 3, Math.min(maxC, 6));
        } catch (eConc) {}
      } else if (background) {
        // iss/70: background lowers budget; do not maximize.
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
          global.__GR_PAGE_HIDING__ = false;
          // Cap concurrency while backgrounded.
          cfg.concurrency = Math.min(Number(cfg.concurrency) || 3, 2);
        } catch (eBg) {}
      }
      // Boost hard anchors still pending so flush prioritizes commercial materials.
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id) || isDeepenBatch(pending[i].batch_id)) {
          pending[i].priority =
            (pending[i].priority || 0) +
            (isHardAnchorBatch(pending[i].batch_id)
              ? cfg.hard_anchor_priority_boost || 1000
              : 200);
          pending[i].hard_anchor_flush = isHardAnchorBatch(pending[i].batch_id);
        }
      }
      pump();
      return api.stats();
    },
    /** Tab visible again — restore full concurrency and clear background flag. */
    markVisible: function () {
      hiding = false;
      try {
        global.__GR_PAGE_BACKGROUNDED__ = false;
        // Never clear true unload flags here (page is going away).
        if (!global.__GR_PAGE_UNLOADING__) {
          global.__GR_PAGE_HIDING__ = false;
        }
      } catch (eV) {}
      applyFirstPartyPerfHints();
      if (!ramped && cfg.concurrency < (cfg.mid_ramp_concurrency || 8)) {
        cfg.concurrency = Math.max(cfg.concurrency, cfg.mid_ramp_concurrency || 8);
      }
      pump();
    },
    markHiding: function () {
      // Legacy: treat as background unless already unloading.
      if (!isPageUnloading()) {
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
        } catch (eM) {}
      } else {
        hiding = true;
      }
    },
    isHardAnchorBatch: isHardAnchorBatch,
    isDeepenBatch: isDeepenBatch,
    isCommercialLandFast: isCommercialLandFast,
    backoffMsForItem: backoffMsForItem,
    isPageUnloading: isPageUnloading,
    isPageBackgrounded: isPageBackgrounded,
    /** Pure: commercial/hard final complete (exported for boot multi-tick). */
    hardFinalComplete: hardFinalComplete,
    parseTerminal: parseTerminal,
    alreadySent: function (item) {
      return !!sentKeys[keyOf(item || {})];
    },
    /** iss/70 P0: batch has frozen capture pending upload/retry (hard-SLA must not re-collect). */
    hasPendingCapture: hasPendingCapture,
    /** iss/70: acked | conflict | uploading | retry_wait | queued | collected | absent */
    materialState: materialState,
    /**
     * Self-heal Loop T: re-pump pending or re-queue frozen capture for a batch.
     * Does not re-collect GPU materials.
     */
    nudgeTransport: function (batchId, sessionId) {
      var bid = String(batchId || "");
      if (!bid) {
        pump();
        return false;
      }
      var sidN = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
      var i;
      // Clear deferred retry so pump can take it now.
      for (i = 0; i < pending.length; i++) {
        if (String(pending[i].batch_id || "") !== bid) continue;
        if (sidN && pending[i].session_id && String(pending[i].session_id) !== sidN) continue;
        try {
          delete pending[i]._retry_after_ms;
        } catch (eClr) {}
      }
      pump();
      return hasPendingCapture(bid, sidN);
    },
    /** Force pump (self-heal / tests). */
    pump: pump,
    freezeCapture: freezeCapture,
    payloadHashOf: payloadHashOf,
    applyAck: applyAck,
    registryGet: registryGet,
    registrySnapshot: function () {
      var out = {};
      Object.keys(materialRegistry).forEach(function (k) {
        out[k] = materialRegistry[k];
      });
      return out;
    },
    effectiveUploadCap: effectiveUploadCap,
    stats: function () {
      var hardPending = 0;
      var pendingBatches = [];
      var seenB = {};
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id)) hardPending++;
        var pb = String((pending[i] && pending[i].batch_id) || "");
        if (pb && !seenB[pb]) {
          seenB[pb] = 1;
          pendingBatches.push(pb);
        }
      }
      return {
        pending: pending.length,
        pending_batches: pendingBatches.slice(0, 32),
        inflight: inflight,
        inflight_items: inflightItems.length,
        concurrency: cfg.concurrency,
        concurrency_max: cfg.concurrency_max,
        effective_cap: effectiveUploadCap(),
        sent: sent,
        failed: failed,
        skipped_dedupe: skipped,
        success_short_circuit: successShortCircuit,
        pagehide_dropped: pagehideDropped,
        soft_abort_on_unload: softAbortOnUnload,
        flush_reason: flushReason,
        hard_anchor_pending: hardPending,
        hard_anchor_flushed: hardFlushed,
        hard_anchor_retries: hardRetries,
        hard_anchor_priority_boost: cfg.hard_anchor_priority_boost || 1000,
        transport_retries: transportRetryCount,
        capture_freezes: captureFreezeCount,
        ack_stored: ackStored,
        ack_duplicate: ackDuplicate,
        ack_merged: ackMerged,
        ack_conflict: ackConflict,
        aimd_ups: aimdUps,
        aimd_downs: aimdDowns,
        registry_n: Object.keys(materialRegistry).length,
        hiding: hiding,
        halted: !!haltState,
        halt_reason: haltState && haltState.reason,
        halt_code: haltState && haltState.code,
        halt_session_id: haltState && haltState.session_id,
        halted_drops: haltedDrops,
      };
    },
    /** Pure helper for tests: sort order under hide with hard boost. */
    sortPreview: function (items, hide) {
      var was = hiding;
      if (hide) hiding = true;
      var copy = (items || []).slice().map(function (it) {
        return {
          batch_id: it.batch_id,
          priority: it.priority || 0,
          effective: effectivePriority(it),
          hard: isHardAnchorBatch(it.batch_id),
        };
      });
      copy.sort(function (a, b) {
        return b.effective - a.effective;
      });
      hiding = was;
      return copy;
    },
    /** Resolve when upload queue is idle (or timeout). Used by multi-tick settle. */
    whenIdle: function (timeoutMs) {
      timeoutMs = timeoutMs == null ? 2000 : timeoutMs;
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          if (pending.length === 0 && inflight === 0) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW) {}
            resolve(api.stats());
            return;
          }
          if (Date.now() - start >= timeoutMs) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW2) {}
            resolve(api.stats());
            return;
          }
          setTimeout(tick, 16);
        }
        tick();
      });
    },
    /** P2: probe depth class for current session outcome. */
    probeDepthClass: function () {
      try {
        var ok = sessionOutcome.batches_ok || {};
        var hasB0 = !!ok["B0_bootstrap"];
        var hasB10 = !!ok["B10_hw_curves"];
        var hasB8 = !!ok["B8_gateway"];
        var n = Object.keys(ok).length;
        if (hasB8 && !hasB0 && n <= 2) return "gateway_only";
        if (hasB0 && hasB10) return "browser_b0_b10";
        if (hasB0) return "browser_partial";
        if (n) return "browser_lite";
        return "empty";
      } catch (e) {
        return "empty";
      }
    },
    /**
     * Progressive land snapshot (window still open).
     * Used by self_heal / ops to drive missing_probe reduction without waiting for unload.
     */
    completenessSnapshot: function () {
      var ok = sessionOutcome.batches_ok || {};
      var hard = cfg.hard_anchor_batches || [];
      var hardOk = [];
      var hardMiss = [];
      for (var i = 0; i < hard.length; i++) {
        if (ok[hard[i]]) hardOk.push(hard[i]);
        else hardMiss.push(hard[i]);
      }
      var okKeys = Object.keys(ok);
      var ver = "";
      try {
        ver = String(
          global.__GR_PRODUCT_VERSION__ ||
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            global.__GR_FE_PACKS_VERSION__ ||
            ""
        );
      } catch (eV) {}
      return {
        progressive: true,
        product_version: ver,
        probe_depth_class: api.probeDepthClass(),
        hard_total: hard.length,
        hard_ok_n: hardOk.length,
        hard_ok: hardOk,
        hard_missing: hardMiss,
        batches_ok_n: okKeys.length,
        batches_ok: okKeys,
        pending: pending.length,
        inflight: inflight,
        halted: !!haltState,
        hiding: !!hiding,
        sealed_ok: sessionOutcome.sealed_ok || 0,
        network_fail: sessionOutcome.network || 0,
        main_complete: !!(ok["B0_bootstrap"] && ok["B10_hw_curves"]),
      };
    },
    /**
     * Keep draining queue while the tab is open (not only pagehide).
     * Safe no-op when empty / halted.
     */
    pumpWhileOpen: function (why) {
      if (haltState) return api.completenessSnapshot();
      try {
        if (typeof api.flush === "function") {
          api.flush(why || "progressive_open");
        }
      } catch (eP) {}
      try {
        pump();
      } catch (e2) {}
      return api.completenessSnapshot();
    },
  };

  global.GRUploadQueue = api;
})(typeof window !== "undefined" ? window : globalThis);

/* ---- pack_loader.js ---- */
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

/* ---- collectors/l1.js ---- */
/**
 * GR L1 — ultra-lite short-visit collectors (v57 mp.l1 aligned).
 * Flat `fields` payloads for greenv5 brain (not nested max-probe sections).
 * Independent of collectors/registry.js — first evidence without 200KB+ module.
 *
 * Public API (stable under minify):
 *   GRL1.collectB0() / collectB1() / collectB2() / collectB3() / collectB12()
 *   GRL1.enqueueBatch(ctx, batchId, fields, priority)
 *   GRL1.runB0(ctx) → Promise
 *   GRL1.runProgressiveRest(ctx) → Promise  // B1,B12,B2,B3 after B0
 *   GRL1.analysisCriticalOrder / priorities / batchIds
 */
(function (global) {
  "use strict";

  function safe(fn, fb) {
    try {
      return fn();
    } catch (e) {
      return fb !== undefined ? fb : null;
    }
  }

  function deriveOsFamily(ua, platform) {
    ua = String(ua || "").toLowerCase();
    platform = String(platform || "").toLowerCase();
    if (/android/.test(platform)) return "android";
    if (/iphone|ipad|ipod/.test(platform)) return "ios";
    if (/win/.test(platform)) return "windows";
    if (/linux|x11/.test(platform)) return "linux";
    if (/mac/.test(platform)) return "macos";
    if (/android/.test(ua)) return "android";
    if (/iphone|ipad|ipod|ios/.test(ua)) return "ios";
    if (/cros/.test(ua)) return "chromeos";
    if (/windows|win32|win64/.test(ua)) return "windows";
    if (/linux|x11/.test(ua)) return "linux";
    if (/mac os x|macintosh|macintel/.test(ua)) return "macos";
    if (platform) return "other";
    return "";
  }

  function formClass(scr, nav) {
    var w = (scr && scr.width) || 0;
    var touch = (nav && nav.maxTouchPoints) || 0;
    if (w > 0 && w < 600) return "mobile";
    if (touch > 1 && w > 0 && w < 900) return "mobile";
    var ua = (nav && nav.userAgent) || "";
    if (/Mobi|Android|iPhone|iPad/i.test(ua) && (touch > 0 || w < 900)) return "mobile";
    return "desktop";
  }

  /**
   * Engine family without InstallTrigger (deprecated in Firefox).
   * Capability-first: chrome object → blink; mozInnerScreenX / real Gecko UA → gecko.
   */
  function engineFamily(ua) {
    ua = ua || "";
    if (!!(global.chrome) || /Chrome\//.test(ua) || /CriOS\//.test(ua) || /Edg\//.test(ua) || /OPR\//.test(ua))
      return "blink";
    try {
      if (typeof global.mozInnerScreenX === "number") return "gecko";
    } catch (e0) {}
    if (/Firefox\//.test(ua) || /FxiOS\//.test(ua)) return "gecko";
    if (/Gecko\//.test(ua) && !/like Gecko/.test(ua)) return "gecko";
    if (global.webkitRequestAnimationFrame || /AppleWebKit\//.test(ua)) return "webkit";
    return "unknown";
  }

  /**
   * Single-shot screen metrics. Firefox Fingerprinting Protection may alter
   * availWidth/Height and log once — cache avoids repeat reads/noise.
   * Emits screen_fp_protection_suspect when values look RFP-spoofed.
   */
  var _l1ScreenCache = null;
  function readScreenMetrics() {
    if (_l1ScreenCache) return _l1ScreenCache;
    var scr = global.screen || {};
    var w = scr.width != null ? scr.width : null;
    var h = scr.height != null ? scr.height : null;
    var aw = scr.availWidth != null ? scr.availWidth : null;
    var ah = scr.availHeight != null ? scr.availHeight : null;
    var rfp = false;
    try {
      if (w != null && h != null && aw != null && ah != null) {
        if (aw === w && ah === h) rfp = true;
        if (w === 1000 && h === 1000) rfp = true;
      }
    } catch (e) {}
    _l1ScreenCache = {
      screen_width: w,
      screen_height: h,
      screen_avail_width: aw,
      screen_avail_height: ah,
      color_depth: scr.colorDepth != null ? scr.colorDepth : null,
      pixel_depth: scr.pixelDepth != null ? scr.pixelDepth : null,
      screen_fp_protection_suspect: rfp,
    };
    return _l1ScreenCache;
  }

  function timezone() {
    return safe(function () {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    }, "");
  }

  function intlLocale() {
    return safe(function () {
      return Intl.DateTimeFormat().resolvedOptions().locale || "";
    }, "");
  }

  /** probe_dag_v2 decision stamp for B0 (design §9.3): version + engine claim
   *  + skip count. Engines are feature claims; UA weight ≈ 0 by design. */
  function dagDecisionStamp() {
    try {
      var consumed = global.__GR_DAG_V2_CONSUMED__ || null;
      if (consumed) {
        var skippedN = 0;
        Object.keys(consumed.skipped || {}).forEach(function () {
          skippedN++;
        });
        return {
          dag_v2_version: consumed.version || 0,
          dag_v2_engine: consumed.engine || "unknown",
          dag_v2_skipped_n: skippedN,
        };
      }
      if (global.GRPackLoader && typeof global.GRPackLoader.dagUploadStamp === "function") {
        return global.GRPackLoader.dagUploadStamp();
      }
    } catch (eD) {}
    return {
      dag_v2_version: 0,
      dag_v2_engine: engineFamily((global.navigator && global.navigator.userAgent) || ""),
      dag_v2_skipped_n: 0,
    };
  }

  /** B0 surface — first evidence for short-visit analyze. */
  function collectB0() {
    var nav = global.navigator || {};
    var sm = readScreenMetrics();
    var scr = { width: sm.screen_width, height: sm.screen_height };
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var dagStamp = dagDecisionStamp();
    return {
      user_agent: ua.slice(0, 500),
      language: nav.language || "",
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: platform,
      vendor: nav.vendor || "",
      os_family: deriveOsFamily(ua, platform),
      form_class: formClass(scr, nav),
      engine_family: engineFamily(ua),
      dag_v2_version: dagStamp.dag_v2_version,
      dag_v2_engine: dagStamp.dag_v2_engine,
      dag_v2_skipped_n: dagStamp.dag_v2_skipped_n,
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: sm.screen_width,
      screen_height: sm.screen_height,
      screen_avail_width: sm.screen_avail_width,
      screen_avail_height: sm.screen_avail_height,
      screen_fp_protection_suspect: sm.screen_fp_protection_suspect,
      color_depth: sm.color_depth,
      pixel_depth: sm.pixel_depth,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      cookie_enabled: !!nav.cookieEnabled,
      timezone: timezone(),
      timezone_offset_min: safe(function () {
        return new Date().getTimezoneOffset();
      }, null),
      intl_locale: intlLocale(),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      outer_width: typeof global.outerWidth !== "undefined" ? global.outerWidth : null,
      outer_height: typeof global.outerHeight !== "undefined" ? global.outerHeight : null,
      inner_width: typeof global.innerWidth !== "undefined" ? global.innerWidth : null,
      inner_height: typeof global.innerHeight !== "undefined" ? global.innerHeight : null,
      webdriver: !!nav.webdriver,
      lite: true,
      race: "l1",
      l1_batch: "B0_bootstrap",
    };
  }

  function collectB1() {
    var nav = global.navigator || {};
    var ua = nav.userAgent || "";
    var f = {
      webdriver: !!nav.webdriver,
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: nav.platform || "",
      user_agent: ua.slice(0, 500),
      automation: {
        webdriver: !!nav.webdriver,
        playwright: !!(
          (nav.webdriver && /HeadlessChrome|Playwright/i.test(ua)) ||
          global._playwright ||
          global.__playwright ||
          global.__pwInitScripts
        ),
        selenium: !!(
          global.document &&
          global.document.documentElement &&
          global.document.documentElement.getAttribute &&
          global.document.documentElement.getAttribute("webdriver")
        ),
        cdc: false,
        phantom: !!(global.callPhantom || global._phantom),
        nightmare: !!global.__nightmare,
      },
      chrome_runtime: !!(global.chrome && global.chrome.runtime),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      headless_ua: /HeadlessChrome|PhantomJS/i.test(ua),
      lite: true,
      race: "l1",
      l1_batch: "B1_conflict",
    };
    try {
      f.automation.cdc = !!(
        global.cdc_adoQpoasnfa76pfcZLmcfl_Array || global.cdc_adoQpoasnfa76pfcZLmcfl_Promise
      );
    } catch (e) {}
    return f;
  }

  function collectB12() {
    var nav = global.navigator || {};
    var scr = global.screen || {};
    return {
      webdriver: !!nav.webdriver,
      chrome_runtime: !!(global.chrome && global.chrome.runtime),
      chrome_app: !!(global.chrome && global.chrome.app),
      plugins_length: nav.plugins ? nav.plugins.length : null,
      mime_types_length: nav.mimeTypes ? nav.mimeTypes.length : null,
      inner_width: typeof global.innerWidth !== "undefined" ? global.innerWidth : null,
      inner_height: typeof global.innerHeight !== "undefined" ? global.innerHeight : null,
      outer_width: typeof global.outerWidth !== "undefined" ? global.outerWidth : null,
      outer_height: typeof global.outerHeight !== "undefined" ? global.outerHeight : null,
      screen_width: scr.width != null ? scr.width : null,
      screen_height: scr.height != null ? scr.height : null,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      notification_permission: safe(function () {
        return typeof Notification !== "undefined" ? Notification.permission : null;
      }, null),
      lite: true,
      race: "l1",
      l1_batch: "B12_anti_camouflage",
    };
  }

  function collectB3() {
    var nav = global.navigator || {};
    var scr = global.screen || {};
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var mq = {};
    try {
      if (global.matchMedia) {
        ["(prefers-color-scheme: dark)", "(pointer: coarse)", "(hover: hover)"].forEach(function (q) {
          try {
            mq[q] = !!global.matchMedia(q).matches;
          } catch (e) {}
        });
      }
    } catch (e2) {}
    return {
      timezone: timezone(),
      timezone_offset_min: safe(function () {
        return new Date().getTimezoneOffset();
      }, null),
      language: nav.language || "",
      languages: safe(function () {
        return Array.prototype.slice.call(nav.languages || [], 0, 8);
      }, []),
      platform: platform,
      os_family: deriveOsFamily(ua, platform),
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: scr.width != null ? scr.width : null,
      screen_height: scr.height != null ? scr.height : null,
      color_depth: scr.colorDepth != null ? scr.colorDepth : null,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      media_queries_lite: mq,
      intl_locale: intlLocale(),
      lite: true,
      race: "l1",
      l1_batch: "B3_system",
    };
  }

  function collectB2() {
    var nav = global.navigator || {};
    var sm = readScreenMetrics();
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var f = {
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      screen_width: sm.screen_width,
      screen_height: sm.screen_height,
      screen_avail_width: sm.screen_avail_width,
      screen_avail_height: sm.screen_avail_height,
      screen_fp_protection_suspect: sm.screen_fp_protection_suspect,
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      color_depth: sm.color_depth,
      device_pixel_ratio: typeof global.devicePixelRatio !== "undefined" ? global.devicePixelRatio : null,
      timezone: timezone(),
      os_family: deriveOsFamily(ua, platform),
      webgl_support: false,
      webgl_vendor: "",
      webgl_renderer: "",
      webgl_unmasked_vendor: "",
      webgl_unmasked_renderer: "",
      webgl2_support: false,
      lite: true,
      race: "l1",
      l1_batch: "B2_hardware",
    };
    try {
      if (typeof document !== "undefined" && document.createElement) {
        var c = document.createElement("canvas");
        c.width = 1;
        c.height = 1;
        var gl =
          c.getContext("webgl") ||
          c.getContext("experimental-webgl") ||
          c.getContext("webgl2");
        var gl2 = null;
        try {
          gl2 = c.getContext("webgl2");
        } catch (e2a) {}
        if (gl) {
          f.webgl_support = true;
          try {
            f.webgl_vendor = gl.getParameter(gl.VENDOR) || "";
            f.webgl_renderer = gl.getParameter(gl.RENDERER) || "";
            f.webgl_version = gl.getParameter(gl.VERSION) || "";
          } catch (eV) {}
          try {
            var dbg = gl.getExtension("WEBGL_debug_renderer_info");
            if (dbg) {
              f.webgl_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) || "";
              f.webgl_unmasked_renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "";
            }
          } catch (eD) {}
          try {
            var ext = gl.getSupportedExtensions();
            f.webgl_extensions_count = ext ? ext.length : 0;
          } catch (eE) {}
          // Class-K needs texture before CIF; L1 B2 is often the only early caps path
          // (registry B2 may be deduped after this batch lands).
          try {
            var tex = null;
            try {
              tex = gl.getParameter(gl.MAX_TEXTURE_SIZE);
            } catch (eT0) {}
            // Fallback enum (0x0D33) if constant missing / governor muted pname.
            if (!(typeof tex === "number" && isFinite(tex) && tex > 0)) {
              try {
                tex = gl.getParameter(0x0d33);
              } catch (eT1) {}
            }
            if (!(typeof tex === "number" && isFinite(tex) && tex > 0) && gl2) {
              try {
                tex = gl2.getParameter(gl2.MAX_TEXTURE_SIZE || 0x0d33);
              } catch (eT2) {}
            }
            if (typeof tex === "number" && isFinite(tex) && tex > 0) {
              f.gl_max_texture_size = tex | 0;
              f.webgl_max_texture = tex | 0;
              f.b2_caps_coland = true;
            }
          } catch (eT) {}
          // Do not loseContext here — kills shared governor pool and later caps probes.
        }
        f.webgl2_support = !!gl2;
      }
    } catch (eG) {}
    return f;
  }

  var PRIORITIES = {
    B0_bootstrap: 120,
    B1_conflict: 110,
    B12_anti_camouflage: 105,
    B2_hardware: 100,
    B3_system: 98,
  };

  /** Progressive after B0: analysis-critical companions (brain main_core / thin_main). */
  var REST_ORDER = ["B1_conflict", "B12_anti_camouflage", "B2_hardware", "B3_system"];

  var COLLECTORS = {
    B0_bootstrap: collectB0,
    B1_conflict: collectB1,
    B12_anti_camouflage: collectB12,
    B2_hardware: collectB2,
    B3_system: collectB3,
  };

  function stampPageId(fields, ctx) {
    var f = fields || {};
    if (f.page_id != null) return f;
    var pid =
      (ctx && ctx.page_id) ||
      (ctx && ctx.fields && ctx.fields.page_id) ||
      global.__GR_PAGE_ID__ ||
      null;
    if (pid == null) return f;
    return Object.assign({}, f, { page_id: pid });
  }

  function enqueueBatch(ctx, batchId, fields, priority) {
    if (!ctx || !ctx.queue || typeof ctx.queue.enqueue !== "function") {
      throw new Error("l1_enqueue_needs_queue");
    }
    var prio = priority != null ? priority : PRIORITIES[batchId] || 90;
    var f = stampPageId(fields, ctx);
    ctx.queue.enqueue({
      session_id: ctx.session_id,
      batch_id: batchId,
      source: "main",
      inject_path: ctx.inject_path,
      priority: prio,
      payload: {
        fields: f,
        sandbox_kind: "main",
        lite: true,
        race: "l1",
      },
    });
    try {
      global.__GR_PACK_KICK_ORDER__ = (global.__GR_PACK_KICK_ORDER__ || []).concat([batchId]);
      global.__GR_L1_KICKED__ = global.__GR_L1_KICKED__ || {};
      global.__GR_L1_KICKED__[batchId] = Date.now();
      if (global.GRPackLoader && GRPackLoader.markKicked) {
        GRPackLoader.markKicked(batchId);
      }
    } catch (eM) {}
    return { batch_id: batchId, priority: prio, lite: true };
  }

  function runOne(ctx, batchId) {
    var fn = COLLECTORS[batchId];
    if (!fn) return Promise.resolve(null);
    return Promise.resolve()
      .then(function () {
        return fn();
      })
      .then(function (fields) {
        return enqueueBatch(ctx, batchId, fields, PRIORITIES[batchId]);
      });
  }

  /** First evidence only. */
  function runB0(ctx) {
    return runOne(ctx, "B0_bootstrap");
  }

  /**
   * Progressive rest after B0 is enqueued (analysis-critical, short-visit).
   * Parallel collect+enqueue (edge collect while B0 upload inflight) — maximizes browser cores/network.
   * Does not load registry. B2 may touch WebGL briefly.
   */
  function runProgressiveRest(ctx) {
    // Fire all rest collectors in parallel; enqueue order is priority-sorted by queue pump.
    return Promise.all(
      REST_ORDER.map(function (id) {
        return runOne(ctx, id).catch(function (e) {
          return { batch_id: id, error: String((e && e.message) || e) };
        });
      })
    ).then(function (rows) {
      return (rows || []).filter(Boolean);
    });
  }

  /** All L1 packs: B0 then rest (for harness / short-visit full lite). */
  function runAllLite(ctx) {
    return runB0(ctx).then(function (b0) {
      return runProgressiveRest(ctx).then(function (rest) {
        return { b0: b0, rest: rest, order: ["B0_bootstrap"].concat(REST_ORDER) };
      });
    });
  }

  var api = {
    collectB0: collectB0,
    collectB1: collectB1,
    collectB2: collectB2,
    collectB3: collectB3,
    collectB12: collectB12,
    enqueueBatch: enqueueBatch,
    runB0: runB0,
    runProgressiveRest: runProgressiveRest,
    runAllLite: runAllLite,
    priorities: PRIORITIES,
    restOrder: REST_ORDER.slice(),
    analysisCriticalOrder: ["B0_bootstrap"].concat(REST_ORDER),
    batchIds: Object.keys(COLLECTORS),
    /** Pure order helper for tests (no DOM). */
    progressivePlan: function () {
      return {
        first: "B0_bootstrap",
        rest: REST_ORDER.slice(),
        // B11 binds early in boot after B0; wave2 is residual heavy only.
        deferred_heavy: ["B10_hw_curves", "B7_sandbox"],
        early_rpa: ["B11_interaction"],
        requires_registry: false,
        requires_full_modules: false,
      };
    },
  };

  global.GRL1 = api;
  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
})(typeof window !== "undefined" ? window : typeof globalThis !== "undefined" ? globalThis : this);

