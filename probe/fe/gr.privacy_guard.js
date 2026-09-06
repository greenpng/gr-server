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
