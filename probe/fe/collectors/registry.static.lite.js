/* green-v5 registry.static.lite — wave1; auto-split; do not edit by hand */
/**
 * Dynamic probe component registry — aligned with spec/component_catalog.json.
 * Static packs race without analyze; dynamic packs only via route_plan apply.
 * Collectors emit fields only — never soft/hard identity decisions.
 */
(function (global) {
  "use strict";

  var collectors = Object.create(null);

  function register(id, def) {
    collectors[id] = def;
  }

  /**
   * Gecko without InstallTrigger (deprecated — typeof alone warns in Firefox).
   * Uses mozInnerScreenX / -moz- CSS + real Gecko UA (not WebKit "like Gecko").
   */
  function isGeckoEngine(ua) {
    ua = String(ua || (navigator && navigator.userAgent) || "");
    try {
      if (typeof window.mozInnerScreenX === "number") return true;
    } catch (e0) {}
    try {
      if (
        typeof CSS !== "undefined" &&
        CSS.supports &&
        CSS.supports("-moz-appearance", "none") &&
        (/Firefox\//.test(ua) || /FxiOS\//.test(ua) || (/Gecko\//.test(ua) && !/like Gecko/.test(ua)))
      ) {
        return true;
      }
    } catch (e1) {}
    if (/Firefox\//.test(ua) || /FxiOS\//.test(ua)) return true;
    if (/Gecko\//.test(ua) && !/like Gecko/.test(ua) && !/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua))
      return true;
    return false;
  }

  /**
   * InstallTrigger existence without evaluating the deprecated binding
   * (avoids Firefox console deprecation). Property check only.
   */
  function installTriggerPresentQuiet() {
    try {
      // Never use typeof InstallTrigger / "in window" — both warn on Firefox.
      return !!(
        typeof window !== "undefined" &&
        Object.prototype.hasOwnProperty.call(window, "InstallTrigger")
      );
    } catch (e) {
      return false;
    }
  }

  /** Avoid Object.getOwnPropertyNames(window) — enumerates deprecated fullScreen/onmoz*. */
  function windowKeysCountQuiet() {
    try {
      var n = 0;
      var sample = [
        "document",
        "navigator",
        "location",
        "chrome",
        "safari",
        "crypto",
        "performance",
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "caches",
        "speechSynthesis",
        "AudioContext",
        "WebSocket",
        "Worker",
        "SharedWorker",
        "OffscreenCanvas",
        "GPU",
        "gpu",
      ];
      var i;
      for (i = 0; i < sample.length; i++) {
        try {
          if (sample[i] in window) n++;
        } catch (e1) {}
      }
      return n;
    } catch (e) {
      return null;
    }
  }

  /**
   * Single-shot screen metrics. Firefox Fingerprinting Protection may alter
   * availWidth/Height and log a console note once — caching prevents storms.
   * screen_fp_protection_suspect marks classic RFP spoof patterns (avail==size).
   */
  var _screenMetricsCache = null;
  function readScreenMetrics(force) {
    if (_screenMetricsCache && !force) return _screenMetricsCache;
    var scr = (typeof screen !== "undefined" && screen) || {};
    var w = scr.width != null ? scr.width : null;
    var h = scr.height != null ? scr.height : null;
    var aw = scr.availWidth != null ? scr.availWidth : null;
    var ah = scr.availHeight != null ? scr.availHeight : null;
    var cd = scr.colorDepth != null ? scr.colorDepth : null;
    var pd = scr.pixelDepth != null ? scr.pixelDepth : null;
    var rfp = false;
    try {
      if (w != null && h != null && aw != null && ah != null) {
        // Classic resistFingerprinting: no taskbar chrome (avail equals full).
        if (aw === w && ah === h) rfp = true;
        // Common max-protection spoof palette
        if (w === 1000 && h === 1000) rfp = true;
      }
    } catch (eR) {}
    _screenMetricsCache = {
      screen_width: w,
      screen_height: h,
      screen_avail_width: aw,
      screen_avail_height: ah,
      color_depth: cd,
      pixel_depth: pd,
      screen_fp_protection_suspect: rfp,
      screen_avail_delta_w: w != null && aw != null ? w - aw : null,
      screen_avail_delta_h: h != null && ah != null ? h - ah : null,
    };
    return _screenMetricsCache;
  }

  /**
   * Engine family for probe-profile selection (not commercial identity).
   * Capability-first, UA fallback. Linux Epiphany/WebKit often lacks safari.pushNotification.
   * Returns: blink | gecko | webkit | unknown
   * Does NOT touch InstallTrigger (deprecated).
   */
  function detectEngineFamily() {
    try {
      var ua = String((navigator && navigator.userAgent) || "");
      var uaL = ua.toLowerCase();
      if (window.safari && window.safari.pushNotification) return "webkit";
      // Chromium: chrome object without WebKit-only Safari token.
      if (window.chrome && (window.chrome.runtime || window.chrome.loadTimes || /Chrome\//.test(ua))) {
        // Edge/Opera still Blink.
        if (!/AppleWebKit\/605/.test(ua) || /Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua)) {
          if (/Chrome\/|Chromium\/|Edg\/|OPR\/|CriOS\//.test(ua) || window.chrome.runtime) {
            return "blink";
          }
        }
      }
      // Gecko (Firefox) — capability + UA, never InstallTrigger.
      if (isGeckoEngine(ua)) return "gecko";
      // Linux WebKitGTK / Epiphany: AppleWebKit/605 + Version/x Safari, no Chrome token.
      if (/AppleWebKit\//.test(ua) && /Safari\//.test(ua) && !/Chrome\/|Chromium\/|CriOS\/|Edg\/|OPR\//.test(ua)) {
        return "webkit";
      }
      if (/Chrome\//.test(ua) || /CriOS\//.test(ua) || /Edg\//.test(ua) || /OPR\//.test(ua)) return "blink";
      if (typeof window.webkitRequestFileSystem === "function" && !window.chrome) return "webkit";
      if (/webkit/.test(uaL) && !/chrome|chromium/.test(uaL)) return "webkit";
      return "unknown";
    } catch (e) {
      return "unknown";
    }
  }

  /** Probe profile name used for method tags (measurement path, not identity). */
  function probeProfileForEngine(engine) {
    engine = engine || detectEngineFamily();
    if (engine === "webkit") return "probe_webkit_v1";
    if (engine === "gecko") return "probe_gecko_v1";
    if (engine === "blink") return "probe_blink_v1";
    return "probe_default_v1";
  }

  /**
   * Generic OS family from platform + UA (no vendor/engine allowlists).
   * Prefer platform when definitive: Playwright WebKit UA often says Macintosh while
   * platform remains "Linux x86_64" — platform is the more honest machine signal.
   */
  function deriveOsFamily(ua, platform) {
    ua = String(ua || "").toLowerCase();
    platform = String(platform || "").toLowerCase();
    // Platform-first (cross-browser same host).
    if (/android/.test(platform)) return "android";
    if (/iphone|ipad|ipod/.test(platform)) return "ios";
    if (/win/.test(platform)) return "windows";
    if (/linux|x11/.test(platform)) return "linux";
    if (/mac/.test(platform)) return "macos";
    // UA fallback when platform empty / generic.
    if (/android/.test(ua)) return "android";
    if (/iphone|ipad|ipod|ios/.test(ua)) return "ios";
    if (/cros/.test(ua)) return "chromeos";
    if (/windows|win32|win64/.test(ua)) return "windows";
    if (/linux|x11/.test(ua)) return "linux";
    if (/mac os x|macintosh|macintel/.test(ua)) return "macos";
    if (platform) return "other";
    return "";
  }

  /**
   * Generic machine-stable signals (cross-browser); no engine allowlists / fixed samples.
   * Mirrors commercial multi-signal practice (GA4 join keys + FingerprintJS-class OS/HW probes):
   * cores, timezone, color, touch, audio rate, avail screen, UA-CH architecture.
   * Soft fields that some engines omit are OK — server hashes only cross-engine core.
   */
  function machineStableSignals() {
    var nav = navigator || {};
    var sm = readScreenMetrics();
    var out = {
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      color_depth: sm.color_depth,
      pixel_depth: sm.pixel_depth,
      device_pixel_ratio: typeof devicePixelRatio !== "undefined" ? devicePixelRatio : null,
      hardware_concurrency: nav.hardwareConcurrency || null,
      device_memory: nav.deviceMemory || null,
      cookie_enabled: !!nav.cookieEnabled,
      pdf_viewer: !!nav.pdfViewerEnabled,
      plugins_length: nav.plugins ? nav.plugins.length : null,
      screen_avail_width: sm.screen_avail_width,
      screen_avail_height: sm.screen_avail_height,
      screen_width: sm.screen_width,
      screen_height: sm.screen_height,
      screen_fp_protection_suspect: sm.screen_fp_protection_suspect,
      screen_avail_delta_w: sm.screen_avail_delta_w,
      screen_avail_delta_h: sm.screen_avail_delta_h,
    };
    try {
      var opts = Intl.DateTimeFormat().resolvedOptions();
      out.timezone = opts.timeZone || "";
      out.intl_locale = opts.locale || "";
      out.intl_calendar = opts.calendar || "";
      out.intl_numbering = opts.numberingSystem || "";
    } catch (e) {
      out.timezone = "";
    }
    // Audio sample rate is system-level, often stable across browsers on same host.
    try {
      var AC = window.AudioContext || window.webkitAudioContext;
      if (AC) {
        var ac = new AC();
        out.audio_sample_rate = ac.sampleRate || null;
        // Base latency / output latency when exposed (system audio stack proxy).
        try {
          if (typeof ac.baseLatency === "number") out.audio_base_latency = Math.round(ac.baseLatency * 1000) / 1000;
          if (typeof ac.outputLatency === "number") out.audio_output_latency = Math.round(ac.outputLatency * 1000) / 1000;
        } catch (eLat) {}
        try {
          if (ac.destination && typeof ac.destination.maxChannelCount === "number") {
            out.audio_max_channel_count = ac.destination.maxChannelCount;
          }
        } catch (eMc) {}
        try { if (ac.close) { var _cl = ac.close(); if (_cl && _cl.catch) _cl.catch(function(){}); } } catch (_eCl) {}
      }
    } catch (e2) {
      out.audio_sample_rate = null;
    }
    // Client Hints (Chromium) — architecture / bitness / platform version when exposed.
    try {
      var uad = nav.userAgentData;
      if (uad) {
        out.ua_mobile = !!uad.mobile;
        out.ua_platform = uad.platform || "";
        if (uad.brands && uad.brands.length) {
          out.ua_brands = uad.brands
            .map(function (b) {
              return (b.brand || "") + ":" + (b.version || "");
            })
            .join(",");
        }
      }
    } catch (eCh) {}
    // Network Information API — real network class (not a fixed table).
    try {
      var conn = nav.connection || nav.mozConnection || nav.webkitConnection;
      if (conn) {
        out.net_effective_type = conn.effectiveType || "";
        out.net_downlink = typeof conn.downlink === "number" ? conn.downlink : null;
        out.net_rtt = typeof conn.rtt === "number" ? conn.rtt : null;
        out.net_save_data = !!conn.saveData;
        out.net_type = conn.type || "";
      }
    } catch (eNet) {}
    // Storage estimate (quota) — OS/disk tier proxy, filled async in deepMachineProbes.
    try {
      if (navigator.storage && navigator.storage.estimate) {
        out.storage_estimate_pending = true;
      }
    } catch (e3) {}
    return out;
  }

  /** Bucket storage quota into coarse class (not a fixed sample library). */
  function storageQuotaClass(bytes) {
    if (bytes == null || !(bytes > 0)) return "";
    // log10-ish tiers: dynamic from actual quota, not hardcoded device table
    var gb = bytes / (1024 * 1024 * 1024);
    if (gb < 1) return "lt1g";
    if (gb < 10) return "1_10g";
    if (gb < 50) return "10_50g";
    if (gb < 200) return "50_200g";
    return "ge200g";
  }

  /**
   * Deeper machine probes via raw browser APIs (async):
   * UA-CH high-entropy, storage.estimate, mediaDevices, speech voices, WebRTC host candidates.
   * No fixed fingerprint sample DB — values measured live each session.
   */
  function deepMachineProbes() {
    var out = {};
    var tasks = [];
    // High-entropy Client Hints (Chromium / Edge) — architecture is machine-level.
    try {
      var uad = navigator.userAgentData;
      if (uad && typeof uad.getHighEntropyValues === "function") {
        tasks.push(
          uad
            .getHighEntropyValues([
              "architecture",
              "bitness",
              "model",
              "platform",
              "platformVersion",
              "fullVersionList",
              "wow64",
            ])
            .then(function (h) {
              if (!h) return;
              // Normalize IA-32 label: UA-CH often returns "x86" on 64-bit hosts (blink).
              var arch = h.architecture || "";
              if (arch === "x86" && (String(h.bitness || "") === "64" || h.wow64)) {
                arch = "x86_64";
              }
              out.architecture = arch;
              out.bitness = h.bitness || "";
              out.ua_model = h.model || "";
              out.platform_version = h.platformVersion || "";
              out.wow64 = !!h.wow64;
              // Canonical ua_ch_* names (iss/18 P0 F-5)
              if (h.architecture) out.ua_ch_architecture = (out.architecture || h.architecture);
              if (h.bitness) out.ua_ch_bitness = h.bitness;
              if (h.model != null) out.ua_ch_model = h.model || "";
              if (h.platform) out.ua_ch_platform = h.platform;
              if (h.platformVersion) out.ua_ch_platform_version = h.platformVersion;
              if (h.mobile != null) out.ua_ch_mobile = !!h.mobile;
              out.ua_ch_wow64 = !!h.wow64;
              if (h.fullVersionList && h.fullVersionList.length) {
                out.ua_ch_full_version_list = (h.fullVersionList || [])
                  .map(function (x) {
                    return (x.brand || "") + "/" + (x.version || "");
                  })
                  .join("|")
                  .slice(0, 256);
              }
            })
            .catch(function () {})
        );
      }
    } catch (e0) {}
    // Storage quota class
    try {
      if (navigator.storage && navigator.storage.estimate) {
        tasks.push(
          navigator.storage.estimate().then(function (est) {
            if (!est) return;
            out.storage_quota = est.quota != null ? est.quota : null;
            out.storage_usage = est.usage != null ? est.usage : null;
            out.storage_quota_class = storageQuotaClass(est.quota);
          }).catch(function () {})
        );
      }
    } catch (e1) {}
    // Media device counts (labels often empty without permission — counts still useful).
    try {
      if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
        tasks.push(
          navigator.mediaDevices.enumerateDevices().then(function (list) {
            var inputs = 0;
            var outputs = 0;
            var video = 0;
            (list || []).forEach(function (d) {
              if (d.kind === "audioinput") inputs++;
              else if (d.kind === "audiooutput") outputs++;
              else if (d.kind === "videoinput") video++;
            });
            out.media_input_count = inputs;
            out.media_output_count = outputs;
            out.media_video_count = video;
            out.media_device_count = (list || []).length;
          }).catch(function () {})
        );
      }
    } catch (e2) {}
    // Speech synthesis voice count (OS locale pack proxy).
    try {
      if (window.speechSynthesis && typeof speechSynthesis.getVoices === "function") {
        var voices = speechSynthesis.getVoices() || [];
        if (voices.length) {
          out.speech_voices_count = voices.length;
        } else {
          tasks.push(
            new Promise(function (resolve) {
              var done = false;
              function finish() {
                if (done) return;
                done = true;
                try {
                  out.speech_voices_count = (speechSynthesis.getVoices() || []).length;
                } catch (e) {
                  out.speech_voices_count = null;
                }
                resolve();
              }
              try {
                speechSynthesis.onvoiceschanged = finish;
              } catch (eV) {}
              setTimeout(finish, 400);
            })
          );
        }
      }
    } catch (e3) {}
    // WebRTC host candidates — **host-only, no public STUN**.
    // Public STUN can trigger Chrome Local Network Access permission prompts and
    // adds network load; commercial host separator only needs local host candidates.
    tasks.push(
      new Promise(function (resolve) {
        try {
          var RTC = window.RTCPeerConnection || window.webkitRTCPeerConnection;
          if (!RTC) {
            resolve();
            return;
          }
          var pc = new RTC({
            iceServers: [],
            iceTransportPolicy: "all",
          });
          var hosts = [];
          var srflx = [];
          var relays = [];
          var typeSet = {};
          var candCount = 0;
          // Host-only gather is local; keep budget short to limit rtc pressure.
          var timer = setTimeout(function () {
            try {
              pc.close();
            } catch (e) {}
            finish();
          }, 1200);
          function finish() {
            clearTimeout(timer);
            // Hash host candidates lightly (don't upload raw LAN IP if policy prefers hash).
            out.webrtc_host_count = hosts.length;
            out.webrtc_srflx_count = srflx.length;
            if (hosts.length) {
              out.webrtc_host_ip_hash = simpleHash(hosts.slice().sort().join("|"));
            }
            if (srflx.length) {
              out.webrtc_srflx_ip_hash = simpleHash(srflx.slice().sort().join("|"));
            }
            // ICE morphology (iss/18 P0 F-6)
            out.ice_has_host = !!typeSet.host;
            out.ice_has_srflx = !!typeSet.srflx;
            out.ice_has_relay = !!typeSet.relay;
            out.ice_candidate_count = candCount;
            out.ice_candidate_types = Object.keys(typeSet).sort().join(",");
            out.has_host = out.ice_has_host;
            out.has_srflx = out.ice_has_srflx;
            out.has_relay = out.ice_has_relay;
            if (!candCount) out.ice_morphology = "none";
            else if (typeSet.host && !typeSet.srflx && !typeSet.relay) out.ice_morphology = "host_only";
            else if (typeSet.relay && !typeSet.host) out.ice_morphology = "relay_heavy";
            else if (typeSet.host && typeSet.srflx) out.ice_morphology = "host_srflx";
            else out.ice_morphology = "mixed";
            out.ice_host_n = hosts.length;
            out.ice_srflx_n = srflx.length;
            out.ice_relay_n = relays.length;
            out.webrtc_relay_count = relays.length;
            resolve();
          }
          pc.onicecandidate = function (ev) {
            if (!ev || !ev.candidate || !ev.candidate.candidate) {
              if (ev && !ev.candidate) finish();
              return;
            }
            var c = ev.candidate.candidate;
            candCount++;
            // candidate:... typ host / srflx / relay
            var m = / typ (host|srflx|relay) /.exec(c);
            var ipm = /([0-9]{1,3}(?:\.[0-9]{1,3}){3}|[a-fA-F0-9:]+)/.exec(c);
            if (!m) return;
            typeSet[m[1]] = true;
            if (!ipm) return;
            if (m[1] === "host") hosts.push(ipm[1]);
            else if (m[1] === "srflx") srflx.push(ipm[1]);
            else if (m[1] === "relay") relays.push(ipm[1]);
          };
          try {
            pc.createDataChannel("gr");
            pc.createOffer()
              .then(function (offer) {
                return pc.setLocalDescription(offer);
              })
              .catch(function () {
                finish();
              });
          } catch (ePc) {
            finish();
          }
        } catch (eW) {
          resolve();
        }
      })
    );
    return Promise.all(tasks).then(function () {
      return out;
    });
  }

  function simpleHash(str) {
    var h = 2166136261;
    str = String(str || "");
    for (var i = 0; i < str.length; i++) {
      h ^= str.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return (h >>> 0).toString(16);
  }

  /**
   * Ordered multipath runner for a probe family.
   * paths: [{ id, run: function()=>value|Promise, ok?: (v)=>bool }]
   * Returns { value, path_id, tried: [{id, ok, err}] }
   */
  function probeWithFallbacks(family, paths) {
    var tried = [];
    var i = 0;
    function next() {
      if (i >= paths.length) {
        return Promise.resolve({
          value: null,
          path_id: null,
          family: family || "",
          tried: tried,
          ok: false,
        });
      }
      var p = paths[i++];
      var id = p.id || "path_" + i;
      var okFn =
        typeof p.ok === "function"
          ? p.ok
          : function (v) {
              return v != null && v !== "" && !(Array.isArray(v) && !v.length);
            };
      try {
        return Promise.resolve(p.run())
          .then(function (v) {
            if (okFn(v)) {
              tried.push({ id: id, ok: true, err: null });
              return {
                value: v,
                path_id: id,
                family: family || "",
                tried: tried,
                ok: true,
              };
            }
            tried.push({ id: id, ok: false, err: "empty_or_invalid" });
            return next();
          })
          .catch(function (e) {
            tried.push({
              id: id,
              ok: false,
              err: String(e && e.message ? e.message : e),
            });
            return next();
          });
      } catch (eSync) {
        tried.push({
          id: id,
          ok: false,
          err: String(eSync && eSync.message ? eSync.message : eSync),
        });
        return next();
      }
    }
    return next();
  }

  /** Rank paths for current engine/form (blink|gecko|webkit × desktop|mobile). */
  function orderPathsForEngine(paths, engine, form) {
    engine = engine || detectEngineFamily();
    form = form || (probeIsMobileLike() ? "mobile" : "desktop");
    // Prefer engine-tagged paths first without dropping others.
    var pref = [];
    var rest = [];
    (paths || []).forEach(function (p) {
      var eng = p.engines || p.engine;
      if (!eng || eng === "*" || (Array.isArray(eng) && eng.indexOf(engine) >= 0) || eng === engine) {
        pref.push(p);
      } else {
        rest.push(p);
      }
    });
    // Mobile: push desktop-only heavy paths last
    if (form === "mobile") {
      pref.sort(function (a, b) {
        var am = (a.form || "").indexOf("mobile") >= 0 ? 0 : 1;
        var bm = (b.form || "").indexOf("mobile") >= 0 ? 0 : 1;
        return am - bm;
      });
    }
    return pref.concat(rest);
  }

  /** Longer stable hash for commercial host separators (v1 simpleHash is 8-hex FNV). */
  function simpleHash16(str) {
    var a = 2166136261;
    var b = 2166136261 ^ 0x9e3779b9;
    str = String(str || "");
    var i;
    for (i = 0; i < str.length; i++) {
      a ^= str.charCodeAt(i);
      a = Math.imul(a, 16777619);
      b ^= str.charCodeAt(str.length - 1 - i);
      b = Math.imul(b, 16777619);
    }
    var ha = (a >>> 0).toString(16);
    var hb = (b >>> 0).toString(16);
    while (ha.length < 8) ha = "0" + ha;
    while (hb.length < 8) hb = "0" + hb;
    return ha + hb;
  }

  /**
   * Multi-path hardware inventory (machine-stable when possible).
   * Each signal tries several APIs; missing API → null field, never throw.
   * R100 modes inspired: net, screen_acc, nav_acc, media enum, gl_param.
   */
  function hardwareInventoryMultiPath() {
    var out = {
      hw_inventory_algo: "gr_hw_inventory_multipath_v1",
      probe_paths: [],
    };
    function path(name, ok, detail) {
      out.probe_paths.push({
        name: name,
        ok: !!ok,
        detail: detail || null,
      });
    }
    // --- Display / screen (primary screen + multi-monitor when available) ---
    // Use cached readScreenMetrics (single avail* touch; FPP-aware flags).
    try {
      var smHw = readScreenMetrics();
      var scr = window.screen || {};
      out.screen_width = smHw.screen_width;
      out.screen_height = smHw.screen_height;
      out.screen_avail_width = smHw.screen_avail_width;
      out.screen_avail_height = smHw.screen_avail_height;
      out.screen_color_depth = smHw.color_depth;
      out.screen_pixel_depth = smHw.pixel_depth;
      out.screen_fp_protection_suspect = smHw.screen_fp_protection_suspect;
      out.device_pixel_ratio =
        typeof window.devicePixelRatio === "number" ? window.devicePixelRatio : null;
      out.display_count = 1;
      path("screen_primary", out.screen_width != null, "screen");
      // Multi-screen: never call getScreenDetails() — it triggers Display/Window Management
      // permission prompts (user redline). Use passive screen.isExtended only.
      if (scr.isExtended != null) {
        out.screen_is_extended = !!scr.isExtended;
        path("screen_is_extended", true, String(!!scr.isExtended));
      } else {
        path("screen_multi_api", false, "skipped_no_permission_prompt");
      }
    } catch (eScr) {
      path("screen_primary", false, String(eScr && eScr.message ? eScr.message : eScr));
    }
    // --- Network Information (not NIC MAC — browser API class) ---
    try {
      var conn =
        navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      if (conn) {
        out.net_effective_type = conn.effectiveType || "";
        out.net_downlink = typeof conn.downlink === "number" ? conn.downlink : null;
        out.net_rtt = typeof conn.rtt === "number" ? conn.rtt : null;
        out.net_type = conn.type || "";
        out.net_save_data = !!conn.saveData;
        path("network_information", true, out.net_type || out.net_effective_type || "ok");
      } else {
        path("network_information", false, "no_api");
      }
    } catch (eNet) {
      path("network_information", false, String(eNet && eNet.message ? eNet.message : eNet));
    }
    // --- Touch / input form ---
    try {
      out.max_touch_points =
        typeof navigator.maxTouchPoints === "number" ? navigator.maxTouchPoints : null;
      out.pointer_coarse =
        typeof matchMedia === "function"
          ? !!matchMedia("(pointer: coarse)").matches
          : null;
      path("input_form", true, "touch=" + out.max_touch_points);
    } catch (eIn) {
      path("input_form", false, null);
    }
    // --- Gamepads (count only) ---
    try {
      if (navigator.getGamepads) {
        var gps = navigator.getGamepads() || [];
        var gn = 0;
        for (var gi = 0; gi < gps.length; gi++) if (gps[gi]) gn++;
        out.gamepad_count = gn;
        path("gamepad", true, "n=" + gn);
      } else {
        path("gamepad", false, "no_api");
      }
    } catch (eGp) {
      path("gamepad", false, null);
    }
    // --- Battery (async optional) ---
    out._battery_p = null;
    try {
      if (navigator.getBattery) {
        out._battery_p = navigator.getBattery().then(function (bat) {
          if (!bat) return {};
          return {
            battery_charging: !!bat.charging,
            battery_level:
              typeof bat.level === "number" ? Math.round(bat.level * 1000) / 1000 : null,
          };
        }).catch(function () {
          return {};
        });
        path("battery", true, "pending");
      } else {
        path("battery", false, "no_api");
      }
    } catch (eBat) {
      path("battery", false, null);
    }
    // --- Media devices inventory: counts only (no labels, no getUserMedia) ---
    out._media_p = null;
    try {
      if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
        out._media_p = navigator.mediaDevices.enumerateDevices().then(function (list) {
          var inputs = 0;
          var outputs = 0;
          var video = 0;
          (list || []).forEach(function (d) {
            if (d.kind === "audioinput") inputs++;
            else if (d.kind === "audiooutput") outputs++;
            else if (d.kind === "videoinput") video++;
          });
          return {
            media_input_count: inputs,
            media_output_count: outputs,
            media_video_count: video,
            media_device_count: (list || []).length,
            media_labels_collected: false,
            media_device_id_hash: null,
          };
        }).catch(function () {
          return {};
        });
        path("media_devices", true, "pending");
      } else {
        path("media_devices", false, "no_api");
      }
    } catch (eMd) {
      path("media_devices", false, null);
    }
    // --- WebGL param surface (fallback when residual fails) ---
    try {
      var c = document.createElement("canvas");
      var gl =
        c.getContext("webgl2") ||
        c.getContext("webgl") ||
        c.getContext("experimental-webgl");
      if (gl) {
        out.gl_api = c.getContext("webgl2") ? "webgl2" : "webgl";
        out.gl_max_texture_size = gl.getParameter(gl.MAX_TEXTURE_SIZE);
        out.gl_max_renderbuffer = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE);
        out.gl_max_vertex_attribs = gl.getParameter(gl.MAX_VERTEX_ATTRIBS);
        out.gl_max_texture_image_units = gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS);
        try {
          out.gl_max_varying_vectors = gl.getParameter(gl.MAX_VARYING_VECTORS);
          out.gl_max_vertex_uniform_vectors = gl.getParameter(gl.MAX_VERTEX_UNIFORM_VECTORS);
          out.gl_max_fragment_uniform_vectors = gl.getParameter(gl.MAX_FRAGMENT_UNIFORM_VECTORS);
          out.gl_max_combined_texture_image_units = gl.getParameter(gl.MAX_COMBINED_TEXTURE_IMAGE_UNITS);
          out.gl_max_cube_map_texture_size = gl.getParameter(gl.MAX_CUBE_MAP_TEXTURE_SIZE);
        } catch (eMax) {}
        try {
          var vpd = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
          if (vpd) {
            out.gl_max_viewport_dims = [vpd[0], vpd[1]];
            out.webgl_max_viewport = [vpd[0], vpd[1]];
          }
        } catch (eVp) {}
        try {
          out.webgl_depth_bits = gl.getParameter(gl.DEPTH_BITS);
          out.webgl_stencil_bits = gl.getParameter(gl.STENCIL_BITS);
          out.webgl_samples = gl.getParameter(gl.SAMPLES);
          out.webgl_alpha_bits = gl.getParameter(gl.ALPHA_BITS);
          out.webgl_red_bits = gl.getParameter(gl.RED_BITS);
          out.webgl_green_bits = gl.getParameter(gl.GREEN_BITS);
          out.webgl_blue_bits = gl.getParameter(gl.BLUE_BITS);
        } catch (eBits) {}
        try {
          var hp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
          var mp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.MEDIUM_FLOAT);
          if (hp) out.gl_high_float = [hp.precision, hp.rangeMin, hp.rangeMax];
          if (mp) out.gl_med_float = [mp.precision, mp.rangeMin, mp.rangeMax];
        } catch (ePrec) {}
        try {
          out.webgl_version = gl.getParameter(gl.VERSION) || "";
          out.webgl_shading_language_version = gl.getParameter(gl.SHADING_LANGUAGE_VERSION) || "";
        } catch (eVer) {}
        var exts = gl.getSupportedExtensions() || [];
        out.webgl_extensions_count = exts.length;
        out.webgl_extensions_hash = simpleHash16(exts.slice().sort().join(","));
        var dbg = gl.getExtension("WEBGL_debug_renderer_info");
        if (dbg) {
          out.webgl_unmasked_renderer =
            gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "";
          out.webgl_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) || "";
        }
        path(
          "webgl_params",
          true,
          out.gl_api +
            "|ext=" +
            exts.length +
            "|d=" +
            (out.webgl_depth_bits != null ? out.webgl_depth_bits : "?") +
            "|v=" +
            (out.gl_max_varying_vectors != null ? out.gl_max_varying_vectors : "?")
        );
      } else {
        path("webgl_params", false, "no_gl");
      }
    } catch (eGl) {
      path("webgl_params", false, String(eGl && eGl.message ? eGl.message : eGl));
    }
    // --- Audio sample rate (OfflineAudio / AudioContext cascade) ---
    try {
      var OAC = window.OfflineAudioContext || window.webkitOfflineAudioContext;
      if (OAC) {
        var oac = new OAC(1, 8, 44100);
        out.audio_sample_rate = oac.sampleRate;
        path("audio_oac_sr", true, String(oac.sampleRate));
      } else if (window.AudioContext || window.webkitAudioContext) {
        var AC = window.AudioContext || window.webkitAudioContext;
        var ac = new AC();
        out.audio_sample_rate = ac.sampleRate;
        try {
          try { if (ac.close) { var _cl2 = ac.close(); if (_cl2 && _cl2.catch) _cl2.catch(function(){}); } } catch (_eCl2) {}
        } catch (eCl) {}
        path("audio_ac_sr", true, String(out.audio_sample_rate));
      } else {
        path("audio_sr", false, "no_api");
      }
    } catch (eAu) {
      path("audio_sr", false, null);
    }
    // --- Hardware concurrency / device memory ---
    try {
      out.hardware_concurrency =
        typeof navigator.hardwareConcurrency === "number"
          ? navigator.hardwareConcurrency
          : null;
      out.device_memory =
        typeof navigator.deviceMemory === "number" ? navigator.deviceMemory : null;
      path(
        "nav_hw",
        out.hardware_concurrency != null,
        "cores=" + out.hardware_concurrency + "|mem=" + out.device_memory
      );
    } catch (eHw) {
      path("nav_hw", false, null);
    }
    return out;
  }

  /** Merge async inventory tails (battery/media/multi-screen). */
  function hardwareInventoryAsyncFill(base) {
    base = base || hardwareInventoryMultiPath();
    var tasks = [];
    if (base._battery_p) {
      tasks.push(
        base._battery_p.then(function (b) {
          Object.keys(b || {}).forEach(function (k) {
            base[k] = b[k];
          });
        })
      );
    }
    if (base._media_p) {
      tasks.push(
        base._media_p.then(function (m) {
          Object.keys(m || {}).forEach(function (k) {
            base[k] = m[k];
          });
        })
      );
    }
    // Intentionally skip getScreenDetails() — permission prompt risk (Display / local network UX).
    return Promise.all(tasks).then(function () {
      delete base._battery_p;
      delete base._media_p;
      return base;
    });
  }

  /**
   * Build a linked WebGL program; returns null if compile/link fails (no useProgram on bad prog).
   * Tries FS sources in order (derivatives → mediump stress → v2 fallback).
   */
  function linkWebglProgram(gl, vsSrc, fsSources) {
    function compile(type, src) {
      var s = gl.createShader(type);
      if (!s) return null;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
        try {
          gl.deleteShader(s);
        } catch (eDel) {}
        return null;
      }
      return s;
    }
    var vs = compile(gl.VERTEX_SHADER, vsSrc);
    if (!vs) return null;
    var i;
    for (i = 0; i < fsSources.length; i++) {
      var fs = compile(gl.FRAGMENT_SHADER, fsSources[i].src);
      if (!fs) continue;
      var prog = gl.createProgram();
      if (!prog) {
        try {
          gl.deleteShader(fs);
        } catch (e1) {}
        continue;
      }
      gl.attachShader(prog, vs);
      gl.attachShader(prog, fs);
      gl.linkProgram(prog);
      try {
        gl.deleteShader(fs);
      } catch (e2) {}
      if (gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        return { prog: prog, algo: fsSources[i].algo || "gr_webgl_residual" };
      }
      try {
        gl.deleteProgram(prog);
      } catch (e3) {}
    }
    try {
      gl.deleteShader(vs);
    } catch (e4) {}
    return null;
  }

  /**
   * Fragment shaders for residual.
   * mode:
   *   float   — legacy hist residual (default)
   *   rint    — integer/fixed-point style ops (cross-engine stability research)
   *   ulp     — long dependent float chain (amplify silicon ULP / micro-arch noise)
   *   noderiv — force no dFdx/dFdy candidates
   *   fma_pair— free vs expanded mul-add delta (iss/54 P1)
   *   denorm  — FTZ / denormal snap ladder (iss/54 P2)
   *   tex_lerp— bilinear quant proxy (iss/54 P3)
   *   interp  — interpolate quant proxy (iss/54 P4)
   */
  function residualFsCandidates(seedMix, preferDeriv, scaleHint, mode) {
    var k =
      typeof seedMix === "number" && isFinite(seedMix) ? seedMix.toFixed(6) : "0.000000";
    var list = [];
    mode = String(mode || "float");
    var sc = scaleHint || 256;

    // --- FMA pair (iss/54 P1) ---
    if (mode === "fma_pair" || mode === "fma") {
      list.push({
        algo: "gr_webgl_residual_fma_pair_v1",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=gl_FragCoord.xy; float k=" +
          k +
          ";" +
          " float a=uv.x*0.731+k; float b=uv.y*0.377+1.13; float c=fract(a*b)*0.917;" +
          " float r_free=a*b+c; float r_exp=a*b; r_exp=r_exp+c; float d=abs(r_free-r_exp);" +
          " float t=fract(d*1e6+r_free*0.17+k);" +
          " gl_FragColor=vec4(t, fract(a*b), fract(b*c+d*1e3), 1.0); }",
      });
      list.push({
        algo: "gr_webgl_residual_fma_pair_v1b",
        src:
          "precision mediump float; void main(){" +
          " vec2 uv=gl_FragCoord.xy/" +
          String(sc) +
          ".0; float k=" +
          k +
          ";" +
          " float a=uv.x*0.731+k; float b=uv.y*0.377+1.13; float c=fract(a*b)*0.917;" +
          " float r1=a*b+c; float r2=(a*b)+c; float d=abs(r1-r2);" +
          " float t=fract(d*1e5+r1*0.31);" +
          " gl_FragColor=vec4(t, fract(t*3.1), fract(c+d*100.0), 1.0); }",
      });
      return list;
    }

    // --- Denorm / FTZ (iss/54 P2) ---
    if (mode === "denorm" || mode === "ftz") {
      list.push({
        algo: "gr_webgl_residual_denorm_v1",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=gl_FragCoord.xy; float k=" +
          k +
          ";" +
          " float x=1.0+fract(uv.x*0.001+uv.y*0.0007+k)*1e-6; float snap=0.0; int i;" +
          " for(i=0;i<200;i++){ x=x*0.1; if(x==0.0 && snap<0.5){ snap=float(i); } }" +
          " float t=snap/200.0; float r=fract(snap*0.137+uv.x*0.01+k);" +
          " gl_FragColor=vec4(t, r, fract(t*r+k), 1.0); }",
      });
      list.push({
        algo: "gr_webgl_residual_denorm_v1b",
        src:
          "precision mediump float; void main(){" +
          " vec2 uv=gl_FragCoord.xy/" +
          String(sc) +
          ".0; float k=" +
          k +
          ";" +
          " float x=1.0+uv.x*1e-5+k*1e-7; float snap=0.0; int i;" +
          " for(i=0;i<120;i++){ x*=0.1; if(x==0.0 && snap<0.5) snap=float(i); }" +
          " float t=snap/120.0;" +
          " gl_FragColor=vec4(t, fract(t*3.7+k), fract(uv.y+snap*0.01), 1.0); }",
      });
      return list;
    }

    // --- Tex lerp analytic (iss/54 P3) ---
    if (mode === "tex_lerp" || mode === "tex") {
      list.push({
        algo: "gr_webgl_residual_tex_lerp_analytic_v1",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=gl_FragCoord.xy; float k=" +
          k +
          ";" +
          " float fx=fract(uv.x*0.031+k*0.1); float fy=fract(uv.y*0.029+k*0.07);" +
          " float t00=0.12+k*0.01; float t10=0.87; float t01=0.43; float t11=0.61+k*0.02;" +
          " float a=mix(t00,t10,fx); float b=mix(t01,t11,fx); float s=mix(a,b,fy);" +
          " float fx2=fract(fx+0.5/256.0); float fy2=fract(fy+0.5/256.0);" +
          " float a2=mix(t00,t10,fx2); float b2=mix(t01,t11,fx2); float s2=mix(a2,b2,fy2);" +
          " float d=abs(s-s2); float t=fract(s*17.0+d*1e4+k);" +
          " gl_FragColor=vec4(t, fract(s), fract(d*1e5), 1.0); }",
      });
      return list;
    }

    // --- Interp diverge proxy (iss/54 P4) ---
    if (mode === "interp" || mode === "interp_diverge") {
      list.push({
        algo: "gr_webgl_residual_interp_v1",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=gl_FragCoord.xy; float k=" +
          k +
          ";" +
          " float f_vs=fract(uv.x*0.137+uv.y*0.091+k);" +
          " float f_fs=fract(uv.x*0.137+uv.y*0.091+k);" +
          " float f_i=fract((uv.x-0.5)*0.137+(uv.y-0.5)*0.091+k);" +
          " float d=abs(f_vs-f_i)+abs(f_fs-f_i); float t=fract(d*1e3+f_fs*13.0);" +
          " gl_FragColor=vec4(t, f_i, fract(f_vs*7.0+k), 1.0); }",
      });
      return list;
    }

    // --- R-int: fixed-point / floor arithmetic (portable WebGL1; cross-engine research) ---
    if (mode === "rint" || mode === "int") {
      // Prefer float-floor fixed-point first (WebGL1-safe). Optional GLSL ES3 path second.
      list.push({
        algo: "gr_webgl_residual_rint_v1",
        src:
          "precision mediump float; void main(){" +
          " vec2 uv=floor(gl_FragCoord.xy);" +
          " float k=" +
          k +
          ";" +
          " float ax=floor(uv.x*(13.0+fract(k*10.0)*7.0)+uv.y*(11.0+fract(k*3.0)*5.0));" +
          " float ay=floor(uv.y*(17.0+fract(k*2.0)*3.0)-uv.x*(7.0+fract(k*5.0)*11.0));" +
          " float n=mod(ax*ax+ay*3.0+k*19.0,65536.0);" +
          " float m=mod(n*97.0+floor(n*0.125)+k*131.0,256.0);" +
          " float t=m/255.0;" +
          " gl_FragColor=vec4(t, fract(t*3.1+k), fract(t*7.3+n/65536.0), 1.0); }",
      });
      list.push({
        algo: "gr_webgl_residual_rint_v1b",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=floor(gl_FragCoord.xy);" +
          " float k=" +
          k +
          ";" +
          " float ax=floor(uv.x*13.0+uv.y*11.0+k*1000.0);" +
          " float ay=floor(uv.y*17.0-uv.x*7.0);" +
          " float n=mod(ax*ax+ay*ay, 65521.0);" +
          " float m=mod(n*251.0+ax, 251.0);" +
          " float t=m/250.0;" +
          " gl_FragColor=vec4(t, fract(t*5.0), fract(t*11.0+k), 1.0); }",
      });
      return list;
    }

    // --- ULP chain: amplify micro-architectural / process variation (silicon-level) ---
    if (mode === "ulp" || mode === "silicon_ulp") {
      list.push({
        algo: "gr_webgl_residual_ulp_chain_v1",
        src:
          "precision highp float; void main(){" +
          " vec2 uv=gl_FragCoord.xy;" +
          " float k=" +
          k +
          ";" +
          " float x=uv.x*(0.001+k*1e-4)+uv.y*(0.0007+k*1e-5)+1.0000001;" +
          " float y=uv.y*(0.0011)+uv.x*(0.0005)+1.0000003;" +
          // long dependent chain — exposes FMA/rounding differences across dies
          " float a=x; int i;" +
          " for(i=0;i<48;i++){" +
          "  a=a*(1.000000119+k*1e-9)+y*1e-8;" +
          "  a=a-floor(a*0.999999881)*1.000000059;" +
          "  a=fract(a*1.000000013+sin(y*0.01+float(i)*0.017)*1e-7);" +
          " }" +
          " float b=fract(a*97.0+y*0.13+k);" +
          " float c=fract(a*b*17.0+k*0.31);" +
          " gl_FragColor=vec4(b,c,fract(a+c),1.0); }",
      });
      list.push({
        algo: "gr_webgl_residual_ulp_chain_v1b",
        src:
          "precision mediump float; void main(){" +
          " vec2 uv=gl_FragCoord.xy/" +
          String(sc) +
          ".0;" +
          " float k=" +
          k +
          ";" +
          " float a=uv.x+uv.y*0.37+k*0.01+1.0;" +
          " int i; for(i=0;i<32;i++){ a=fract(a*(1.0001+k*0.00001)+uv.y*0.0001); }" +
          " gl_FragColor=vec4(a, fract(a*3.7), fract(a*11.3+k), 1.0); }",
      });
      return list;
    }

    // float default (+ optional deriv)
    if (preferDeriv && mode !== "noderiv") {
      list.push({
        algo: "gr_webgl_residual_hist_v3d",
        src:
          "#extension GL_OES_standard_derivatives : enable\n" +
          "precision mediump float; void main(){" +
          " vec2 uv=gl_FragCoord.xy;" +
          " float k=" +
          k +
          ";" +
          " float ax=uv.x*(0.137+k*0.01)+uv.y*(0.091+k*0.007);" +
          " float ay=uv.y*(0.173+k*0.009)-uv.x*(0.053+k*0.005);" +
          " float n=sin(ax*1.7)*cos(ay*2.3)+sin((ax+ay)*(3.1+k));" +
          " float m=fract(pow(abs(n)+1.001,1.61)*(97.3+k*3.0));" +
          " float d=abs(dFdx(m))+abs(dFdy(m));" +
          " float t=fract(m*17.0+d*41.0+k);" +
          " gl_FragColor=vec4(t, fract(t*3.7+d*2.1), fract(m+d), 1.0); }",
      });
    }
    list.push({
      algo: "gr_webgl_residual_hist_v3b",
      src:
        "precision mediump float; void main(){" +
        " vec2 uv=gl_FragCoord.xy;" +
        " float k=" +
        k +
        ";" +
        " float ax=uv.x*(0.137+k*0.01)+uv.y*(0.091+k*0.007);" +
        " float ay=uv.y*(0.173+k*0.009)-uv.x*(0.053+k*0.005);" +
        " float n=sin(ax*1.7)*cos(ay*2.3)+sin((ax+ay)*(3.1+k));" +
        " float m=fract(pow(abs(n)+1.001,1.61)*(97.3+k*3.0));" +
        " float t=fract(m*17.0+k+uv.x*uv.y*0.00031);" +
        " gl_FragColor=vec4(t, fract(t*3.7+m*2.1), fract(m+t), 1.0); }",
    });
    // v2 fallback uses scaleHint for uv normalize (256 or 64)
    list.push({
      algo: "gr_webgl_residual_hist_v2",
      src:
        "precision mediump float; void main(){" +
        " vec2 uv=gl_FragCoord.xy/" +
        String(sc) +
        ".0;" +
        " float k=" +
        k +
        ";" +
        " float n=sin(uv.x*(37.1+k))*cos(uv.y*(29.3+k*0.7))+sin((uv.x+uv.y)*(53.7+k*1.3));" +
        " gl_FragColor=vec4(n*0.5+0.5, uv.x, uv.y, 1.0); }",
    });
    return list;
  }

  /**
   * Single v3f residual measurement with explicit options (multi-path building block).
   * opts: { path_id, size, warm_frames, prefer_deriv, force_deriv, ctx_pref, extra_draw, shader_mode }
   * shader_mode: float|rint|ulp|noderiv
   * ctx_pref: "webgl2"|"webgl"|"experimental"|"auto"
   */
  function webglResidualCurveV3fRun(opts) {
    opts = opts || {};
    var t0 = typeof performance !== "undefined" ? performance.now() : Date.now();
    var pathId = opts.path_id || "v3f_default";
    var size = opts.size || 128;
    var warmFrames = opts.warm_frames != null ? opts.warm_frames : 0;
    var shaderMode = String(opts.shader_mode || "float");
    var engine = detectEngineFamily();
    var out = {
      path_id: pathId,
      size: size,
      warm_frames: warmFrames,
      shader_mode: shaderMode,
      engine: engine,
      ok: false,
      curve: null,
      mean: null,
      std: null,
      residual_algo: null,
      ctx: null,
      has_deriv: null,
      err: null,
      elapsed_ms: null,
      eu_timing_ms: null,
    };
    try {
      // Chrome hard-caps ~8–16 live WebGL contexts. Always free ALL tracked
      // probe contexts before creating another (lab: thousands of
      // "Too many active WebGL contexts" warnings = architecture failure).
      try {
        if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
        else releaseWebglProbeContexts();
      } catch (ePreRel) {}
      var c = document.createElement("canvas");
      c.width = size;
      c.height = size;
      var glOpts = { antialias: false, preserveDrawingBuffer: true };
      var gl = null;
      var pref = opts.ctx_pref || "auto";
      if (pref === "webgl2" || pref === "auto") {
        try {
          gl = c.getContext("webgl2", glOpts);
          if (gl) out.ctx = "webgl2";
        } catch (e2) {
          gl = null;
        }
      }
      if (!gl && (pref === "webgl" || pref === "auto" || pref === "webgl2")) {
        try {
          gl = c.getContext("webgl", glOpts);
          if (gl) out.ctx = "webgl";
        } catch (e1) {
          gl = null;
        }
      }
      if (!gl && (pref === "experimental" || pref === "auto")) {
        try {
          gl = c.getContext("experimental-webgl", glOpts);
          if (gl) out.ctx = "experimental-webgl";
        } catch (e0) {
          gl = null;
        }
      }
      if (!gl) {
        out.err = "no_context";
        out.elapsed_ms = Math.round(
          (typeof performance !== "undefined" ? performance.now() : Date.now()) - t0
        );
        return out;
      }
      var hasDeriv = false;
      try {
        hasDeriv = !!gl.getExtension("OES_standard_derivatives");
      } catch (eExt) {
        hasDeriv = false;
      }
      out.has_deriv = hasDeriv;
      var preferDeriv = !!opts.prefer_deriv;
      if (opts.force_deriv === true) preferDeriv = true;
      if (opts.force_deriv === false) preferDeriv = false;
      if (opts.prefer_deriv == null && opts.force_deriv == null) {
        // default: use extension when available (blink/gecko); webkit noderiv paths set force_deriv false
        preferDeriv = hasDeriv;
      }
      var vsSrc = "attribute vec2 a; void main(){ gl_Position=vec4(a,0.0,1.0); }";
      var seeds = [0.0, 0.41, 1.17, 2.53];
      if (opts.seed_k != null && isFinite(Number(opts.seed_k))) {
        // Seeded replay / challenge path: single k derived from session/challenge_seed
        seeds = [Number(opts.seed_k)];
      }
      var strips = 6;
      var curve = [];
      var lastAlgo = "gr_webgl_residual_std_v3f";
      var buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
      // Warmup draws (all engines when warm_frames>0)
      if (warmFrames > 0) {
        try {
          var warmLinked = linkWebglProgram(
            gl,
            vsSrc,
            residualFsCandidates(0, preferDeriv && hasDeriv, size, shaderMode)
          );
          if (warmLinked) {
            gl.useProgram(warmLinked.prog);
            var wloc = gl.getAttribLocation(warmLinked.prog, "a");
            if (wloc >= 0) {
              gl.enableVertexAttribArray(wloc);
              gl.vertexAttribPointer(wloc, 2, gl.FLOAT, false, 0, 0);
              gl.viewport(0, 0, size, size);
              var wi;
              for (wi = 0; wi < warmFrames; wi++) {
                gl.clearColor(0, 0, 0, 1);
                gl.clear(gl.COLOR_BUFFER_BIT);
                gl.drawArrays(gl.TRIANGLES, 0, 3);
              }
              try {
                gl.finish();
              } catch (eFin) {}
            }
            try {
              gl.deleteProgram(warmLinked.prog);
            } catch (eDw) {}
          }
          try {
            hasDeriv = !!gl.getExtension("OES_standard_derivatives");
            out.has_deriv = hasDeriv;
          } catch (eExt2) {}
        } catch (eWarm) {}
      }
      var useDeriv = preferDeriv && hasDeriv;
      if (opts.force_deriv === false || shaderMode === "noderiv" || shaderMode === "rint") {
        useDeriv = false;
      }
      // DrawnApart-lite: multi-draw timing vector (silicon EU variation) when timing_samples set
      var timingSamples = opts.timing_samples != null ? opts.timing_samples | 0 : 0;
      var timingCurve = [];
      if (timingSamples > 0) {
        try {
          var tProg = linkWebglProgram(
            gl,
            vsSrc,
            residualFsCandidates(0.41, false, size, "ulp")
          );
          if (tProg) {
            gl.useProgram(tProg.prog);
            var tLoc = gl.getAttribLocation(tProg.prog, "a");
            if (tLoc >= 0) {
              gl.enableVertexAttribArray(tLoc);
              gl.vertexAttribPointer(tLoc, 2, gl.FLOAT, false, 0, 0);
              gl.viewport(0, 0, size, size);
              var ti;
              for (ti = 0; ti < timingSamples; ti++) {
                var tA =
                  typeof performance !== "undefined" ? performance.now() : Date.now();
                var rep;
                for (rep = 0; rep < 8; rep++) {
                  gl.clearColor(0, 0, 0, 1);
                  gl.clear(gl.COLOR_BUFFER_BIT);
                  gl.drawArrays(gl.TRIANGLES, 0, 3);
                }
                try {
                  gl.finish();
                } catch (eTf) {}
                var tB =
                  typeof performance !== "undefined" ? performance.now() : Date.now();
                timingCurve.push(Math.round((tB - tA) * 1000) / 1000);
              }
            }
            try {
              gl.deleteProgram(tProg.prog);
            } catch (eTd) {}
          }
        } catch (eTim) {}
      }
      var seedI;
      for (seedI = 0; seedI < seeds.length; seedI++) {
        var linked = linkWebglProgram(
          gl,
          vsSrc,
          residualFsCandidates(seeds[seedI], useDeriv, size, shaderMode)
        );
        if (!linked) continue;
        lastAlgo = linked.algo || lastAlgo;
        gl.useProgram(linked.prog);
        var loc = gl.getAttribLocation(linked.prog, "a");
        if (loc < 0) continue;
        gl.enableVertexAttribArray(loc);
        gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
        // Match viewport to drawing buffer to avoid "destination rect smaller than viewport".
        try {
          if (gl.canvas) {
            if (gl.canvas.width !== size) gl.canvas.width = size;
            if (gl.canvas.height !== size) gl.canvas.height = size;
          }
        } catch (eSz) {}
        gl.viewport(0, 0, size, size);
        gl.clearColor(0, 0, 0, 1);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        var extra = opts.extra_draw != null ? opts.extra_draw : engine === "webkit" ? 1 : 0;
        if (extra > 0) {
          var ed;
          for (ed = 0; ed < extra; ed++) {
            try {
              gl.drawArrays(gl.TRIANGLES, 0, 3);
            } catch (eEd) {}
          }
          try {
            gl.finish();
          } catch (eW2) {}
        }
        var px = new Uint8Array(size * size * 4);
        // In-bounds read only (out-of-bounds readPixels is deprecated/slow).
        var rw = size;
        var rh = size;
        try {
          if (gl.drawingBufferWidth > 0) rw = Math.min(size, gl.drawingBufferWidth);
          if (gl.drawingBufferHeight > 0) rh = Math.min(size, gl.drawingBufferHeight);
        } catch (eDb) {}
        gl.readPixels(0, 0, rw, rh, gl.RGBA, gl.UNSIGNED_BYTE, px);
        var stripH = Math.floor(size / strips) || 1;
        var gSum = 0;
        var gSum2 = 0;
        var gN = 0;
        var t;
        for (t = 0; t < strips; t++) {
          var y0 = t * stripH;
          var y1 = t === strips - 1 ? size : y0 + stripH;
          var sum = 0;
          var sum2 = 0;
          var n = 0;
          var y;
          for (y = y0; y < y1; y++) {
            var x;
            for (x = 0; x < size; x += 2) {
              var off = (y * size + x) * 4;
              var v = (px[off] + px[off + 1] * 0.35) / (255 * 1.35);
              sum += v;
              sum2 += v * v;
              n++;
              gSum += v;
              gSum2 += v * v;
              gN++;
            }
          }
          if (n < 1) {
            curve.push(0);
            continue;
          }
          var mean = sum / n;
          var varr = sum2 / n - mean * mean;
          if (varr < 0) varr = 0;
          curve.push(Math.round(Math.sqrt(varr) * 1e5) / 1e5);
        }
        if (gN > 0) {
          var gm = gSum / gN;
          var gv = gSum2 / gN - gm * gm;
          if (gv < 0) gv = 0;
          curve.push(Math.round(gm * 1e5) / 1e5);
          curve.push(Math.round(Math.sqrt(gv) * 1e5) / 1e5);
        } else {
          curve.push(0);
          curve.push(0);
        }
        try {
          gl.deleteProgram(linked.prog);
        } catch (eDel) {}
      }
      if (curve.length < 8) {
        out.err = "curve_short";
        out.elapsed_ms = Math.round(
          (typeof performance !== "undefined" ? performance.now() : Date.now()) - t0
        );
        return out;
      }
      while (curve.length < 32) curve.push(0);
      if (curve.length > 32) curve = curve.slice(0, 32);
      var rSum = 0;
      var rSum2 = 0;
      var rN = 0;
      var ri;
      for (ri = 0; ri < curve.length; ri++) {
        var rv = Number(curve[ri]);
        if (!isNaN(rv) && isFinite(rv)) {
          rSum += rv;
          rSum2 += rv * rv;
          rN++;
        }
      }
      var rMean = rN > 0 ? rSum / rN : null;
      var rStd = null;
      if (rN > 0) {
        var rVar = rSum2 / rN - rMean * rMean;
        if (rVar < 0) rVar = 0;
        rStd = Math.sqrt(rVar);
      }
      out.ok = true;
      out.curve = curve;
      out.mean = rMean != null ? Math.round(rMean * 1e12) / 1e12 : null;
      out.std = rStd != null ? Math.round(rStd * 1e12) / 1e12 : null;
      // Fine ladder (silicon): keep higher resolution stats for conf / same-model split
      out.mean_fine = rMean != null ? Math.round(rMean * 1e6) / 1e6 : null;
      out.std_fine = rStd != null ? Math.round(rStd * 1e6) / 1e6 : null;
      if (timingCurve && timingCurve.length >= 4) {
        out.eu_timing_ms = timingCurve;
        var tSum = 0;
        var tI;
        for (tI = 0; tI < timingCurve.length; tI++) tSum += timingCurve[tI];
        out.eu_timing_mean_ms =
          Math.round((tSum / timingCurve.length) * 1000) / 1000;
      }
      out.residual_algo =
        "gr_webgl_residual_" + shaderMode + "_v3f+" + lastAlgo;
      out.elapsed_ms = Math.round(
        (typeof performance !== "undefined" ? performance.now() : Date.now()) - t0
      );
      // Always track then release immediately — never leave >1 live context.
      try {
        global.__GR_LAST_GL__ = gl;
        global.__GR_PROBE_GL__ = global.__GR_PROBE_GL__ || [];
        global.__GR_PROBE_GL__.push({ gl: gl, canvas: c });
        global.__GR_PROBE_CANVASES__ = global.__GR_PROBE_CANVASES__ || [];
        global.__GR_PROBE_CANVASES__.push(c);
      } catch (eKeep) {}
      // keep_gl is ignored for product safety — business pages must not accumulate GL.
      try {
        if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
        else releaseWebglProbeContexts();
      } catch (eRelAll) {}
      return out;
    } catch (e) {
      out.err = String(e && e.message ? e.message : e);
      out.elapsed_ms = Math.round(
        (typeof performance !== "undefined" ? performance.now() : Date.now()) - t0
      );
      return out;
    }
  }

  /** Lose oldest N (or all) tracked probe GL contexts. */
  function releaseWebglProbeContexts(nOldest) {
    var list = global.__GR_PROBE_GL__ || [];
    var n = nOldest == null ? list.length : Math.max(0, nOldest | 0);
    var i;
    for (i = 0; i < n && list.length; i++) {
      var ent = list.shift();
      try {
        var g = ent && ent.gl;
        if (g) {
          try {
            var lose = g.getExtension && g.getExtension("WEBGL_lose_context");
            if (lose) /*lose_suppressed*/void 0;
          } catch (eL) {}
          // Best-effort: drop program/buffer references so GC can reclaim GPU mem.
          try {
            if (g.getParameter && g.CURRENT_PROGRAM) {
              g.useProgram(null);
            }
          } catch (eU) {}
        }
      } catch (e0) {}
      try {
        if (ent && ent.canvas) {
          ent.canvas.width = 1;
          ent.canvas.height = 1;
        }
      } catch (eZ) {}
    }
    global.__GR_PROBE_GL__ = list;
    global.__GR_PROBE_CANVASES__ = list.map(function (e) {
      return e.canvas;
    });
    if (!list.length) {
      global.__GR_LAST_GL__ = null;
      global.__GR_PROBE_CANVASES__ = [];
    }
  }

  /**
   * Public helper for mid/dense/B10x: acquire one WebGL after releasing all.
   * Callers must not keep the context across async boundaries without holding HW lock.
   */
  function acquireProbeGl(size, opts) {
    opts = opts || {};
    try {
      releaseWebglProbeContexts();
    } catch (eR) {}
    var c = document.createElement("canvas");
    var w = size || 4;
    c.width = w;
    c.height = w;
    var glOpts = opts.glOpts || { antialias: false, preserveDrawingBuffer: false };
    var gl = null;
    try {
      gl = c.getContext("webgl2", glOpts) || c.getContext("webgl", glOpts) || c.getContext("experimental-webgl", glOpts);
    } catch (eG) {
      gl = null;
    }
    if (!gl) return null;
    global.__GR_LAST_GL__ = gl;
    global.__GR_PROBE_GL__ = global.__GR_PROBE_GL__ || [];
    global.__GR_PROBE_GL__.push({ gl: gl, canvas: c });
    global.__GR_PROBE_CANVASES__ = global.__GR_PROBE_CANVASES__ || [];
    global.__GR_PROBE_CANVASES__.push(c);
    return { gl: gl, canvas: c };
  }

  /** Magrank quanta vector (top-8 abs / 0.02) for path agreement scoring. */
  function residualMagrankKey(curve) {
    if (!curve || !curve.length) return "";
    var mags = [];
    var i;
    for (i = 0; i < curve.length; i++) mags.push(Math.abs(Number(curve[i]) || 0));
    mags.sort(function (a, b) {
      return b - a;
    });
    var parts = [];
    for (i = 0; i < 8 && i < mags.length; i++) {
      parts.push(String(Math.round(mags[i] / 0.02)));
    }
    return parts.join(",");
  }

  /**
   * Entropy-ish gate (mirror server tightened gate loosely): non-flat, enough variance.
   */
  function residualPathEntropyOk(curve) {
    if (!curve || curve.length < 8) return false;
    var sum = 0;
    var sum2 = 0;
    var n = 0;
    var i;
    for (i = 0; i < curve.length; i++) {
      var v = Number(curve[i]);
      if (isNaN(v) || !isFinite(v)) continue;
      sum += v;
      sum2 += v * v;
      n++;
    }
    if (n < 8) return false;
    var mean = sum / n;
    var varr = sum2 / n - mean * mean;
    if (varr < 0) varr = 0;
    var std = Math.sqrt(varr);
    if (std < 0.01 && Math.abs(mean) < 0.05) return false;
    if (std < 1e-6) return false;
    return true;
  }

  /**
   * Multi-path residual for **all** engines (async, yields between paths — no UI freeze).
   * @param {object} [opts]
   * @param {string} [opts.profile] default|webkit_deep|softgl|legacy_webgl1|unknown|angle_cross
   * Returns Promise resolving to multipath result. Server re-selects from residual_paths.
   * Profiles only **add** routes — never delete other engines' plans.
   */
  function webglResidualMultiPath(opts) {
    opts = opts || {};
    var profile = String(opts.profile || "default");
    var engine = detectEngineFamily();
    var weak = false;
    try {
      weak = typeof probeIsWeakDevice === "function" ? !!probeIsWeakDevice() : false;
      if (!weak && navigator.hardwareConcurrency && navigator.hardwareConcurrency <= 4) weak = true;
    } catch (eW) {}
    // Multipath budget: serial create+release per path (GL governor).
    // Commercial silicon needs ≥3 independent shader modes (float/noderiv/rint/ulp).
    // Floor never collapses to 1 under live-context pressure — soft-release first.
    // iss/67 B2: healthy desktop default PATH_CAP=5 so Lane-S denorm is not squeezed out
    // by roleOrder noderiv→float→fma→rint (cap4 left denorm as 5th and dropped it).
    var PATH_CAP = weak ? 2 : 5;
    try {
      if (global.GRGlGovernor) {
        if (typeof GRGlGovernor.releaseSoft === "function") {
          try {
            GRGlGovernor.releaseSoft();
          } catch (eRel0) {}
        }
        if (typeof GRGlGovernor.pathCap === "function") {
          var govCap = GRGlGovernor.pathCap(weak);
          if (govCap != null && govCap > 0) PATH_CAP = govCap;
        }
      }
    } catch (eCap) {}
    // Hard floor: weak≥2, healthy≥3 (was path_cap=1 → res/wg global floor)
    // Max 5 so commercial can land fma+denorm without starving Lane-C (float/noderiv/rint).
    var PATH_FLOOR = weak ? 2 : 3;
    PATH_CAP = Math.max(PATH_FLOOR, Math.min(5, PATH_CAP | 0));
    // Commercial multipath v2 (v5.8.137): Lane-C (float/noderiv/rint) + Lane-S (fma/denorm).
    // Root-cause: float+noderiv+rint+ulp alone left same-SKU class floors on au/wg;
    // fma/denorm lived only in B10x silicon_deep (often zero_silicon on gateway_only).
    // B10x still deepens dedicated silicon packs; do not exceed PATH_CAP here.
    var plans;
    if (profile === "webkit_wave2") {
      // Distinct from webkit_deep — was exact duplicate (same curve hash every session).
      plans = [
        { path_id: "w2_warm6_noderiv", size: 128, warm_frames: 6, force_deriv: false, ctx_pref: "webgl", extra_draw: 2, shader_mode: "noderiv" },
        { path_id: "w2_warm8_float", size: 128, warm_frames: 8, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 2 },
        { path_id: "w2_rint_warm4", size: 128, warm_frames: 4, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "rint" },
        { path_id: "w2_ulp_chain", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp" },
      ];
      if (weak) plans = plans.slice(0, 2);
    } else if (profile === "webkit_deep" || (profile === "default" && engine === "webkit")) {
      // Finish-safe warm2 + commercial mode spread incl. fma (WebGL1-safe old GPU).
      // See docs/ARCH_ENGINE_AWARE_PROBE_ANALYSIS_V1.md (WebKitGTK / Epiphany).
      plans = [
        { path_id: "warm2_v3f_128_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "noderiv" },
        { path_id: "warm2_v3f_128_deriv", size: 128, warm_frames: 2, force_deriv: true, ctx_pref: "webgl", extra_draw: 1 },
        { path_id: "webkit_rint_warm2", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "rint" },
        { path_id: "webkit_fma_webgl1", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "fma_pair" },
        { path_id: "webkit_denorm_ftz", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "denorm" },
      ];
      if (profile === "webkit_deep" && !weak) {
        plans.push(
          { path_id: "warm4_v3f_128", size: 128, warm_frames: 4, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 1 },
          { path_id: "warm2_webgl2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl2", extra_draw: 1 }
        );
      }
      if (weak) plans = plans.slice(0, 2);
    } else if (profile === "softgl") {
      plans = [
        { path_id: "soft_warm4_v3f_128", size: 128, warm_frames: 4, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "soft_v3f_128_std", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "soft_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "soft_rint", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "soft_webgl2_128", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl2", extra_draw: 0 },
      ];
      if (weak) plans = plans.slice(0, 3);
    } else if (profile === "legacy_webgl1") {
      // Old WebGL1-only: role-spread for fusion without requiring WebGL2.
      plans = [
        { path_id: "legacy_webgl1_warm2", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "legacy_webgl1_warm4", size: 128, warm_frames: 4, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 1 },
        { path_id: "legacy_webgl1_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "legacy_webgl1_rint", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "legacy_webgl1_ulp", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp" },
      ];
      if (weak) plans = plans.slice(0, 3);
    } else if (profile === "unknown") {
      plans = [
        { path_id: "unk_warm2_webgl", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "unk_warm2_webgl2", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl2", extra_draw: 0 },
        { path_id: "unk_warm4_std", size: 128, warm_frames: 4, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "unk_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "unk_rint", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
      ];
      if (weak) plans = plans.slice(0, 3);
    } else if (profile === "angle_cross") {
      plans = [
        { path_id: "v3f_128_std", size: 128, warm_frames: 0, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "v3f_128_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "rint_warm2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "ulp_chain_128", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp" },
        { path_id: "warm2_webgl2_128", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl2", extra_draw: 0 },
      ];
    } else if (profile === "silicon_rint") {
      // Cross-engine stability research: integer/fixed ops, no deriv
      plans = [
        { path_id: "rint_warm2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "rint_warm4_128", size: 128, warm_frames: 4, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "rint" },
        { path_id: "rint_webgl2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl2", extra_draw: 0, shader_mode: "rint" },
      ];
    } else if (profile === "silicon_noderiv") {
      plans = [
        { path_id: "noderiv_hard_warm2", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "noderiv" },
        { path_id: "noderiv_hard_warm4", size: 128, warm_frames: 4, force_deriv: false, ctx_pref: "webgl", extra_draw: 2, shader_mode: "noderiv" },
        { path_id: "noderiv_hard_webgl2", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl2", extra_draw: 1, shader_mode: "noderiv" },
      ];
    } else if (profile === "silicon_ulp") {
      // Silicon-level: ULP chain + EU timing (DrawnApart-inspired, true GPU noise)
      plans = [
        { path_id: "ulp_chain_128", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp" },
        { path_id: "ulp_chain_warm4", size: 128, warm_frames: 4, force_deriv: false, ctx_pref: "webgl", extra_draw: 1, shader_mode: "ulp" },
        { path_id: "ulp_eu_timing", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp", timing_samples: 14 },
        { path_id: "ulp_chain_webgl2", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl2", extra_draw: 0, shader_mode: "ulp" },
      ];
    } else if (profile === "silicon_deep") {
      // iss/54 P1–P4 advanced silicon (must stay in registry.js SSOT — split regenerates static)
      // WebGL1-first fma_pair so old GPUs without WebGL2 still contribute Lane-S; webgl2 path follows.
      plans = [
        { path_id: "fma_pair_webgl1", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "fma_pair" },
        { path_id: "fma_pair_128", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl2", extra_draw: 0, shader_mode: "fma_pair" },
        { path_id: "denorm_ftz_128", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "denorm" },
        { path_id: "tex_lerp_128", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "tex_lerp" },
        { path_id: "interp_diverge_128", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "interp" },
        { path_id: "ulp_eu_timing_deep", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "ulp", timing_samples: 14 },
      ];
    } else if (engine === "gecko") {
      // float + noderiv + rint + fma (WebGL1-safe) + denorm (PATH_CAP slices)
      plans = [
        { path_id: "v3f_128_std", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "v3f_128_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "rint_warm2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "fma_pair_webgl1", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "fma_pair" },
        { path_id: "denorm_ftz_128", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "denorm" },
      ];
    } else {
      // Blink / Chromium commercial default: Lane-C + fma/denorm (old GPU via webgl1 fma)
      plans = [
        { path_id: "v3f_128_std", size: 128, warm_frames: 2, prefer_deriv: true, ctx_pref: "webgl", extra_draw: 0 },
        { path_id: "v3f_128_noderiv", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "noderiv" },
        { path_id: "rint_warm2_128", size: 128, warm_frames: 2, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "rint" },
        { path_id: "fma_pair_webgl1", size: 128, warm_frames: 1, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "fma_pair" },
        { path_id: "denorm_ftz_128", size: 128, warm_frames: 0, force_deriv: false, ctx_pref: "webgl", extra_draw: 0, shader_mode: "denorm" },
      ];
    }

    // Hard path cap after all profile selection (governor-aware, max 5).
    // Prefer role spread under cap: keep noderiv + at least one of fma/denorm when healthy.
    if (plans && plans.length > PATH_CAP) {
      if (!weak && PATH_CAP >= 4) {
        var roleOf = function (pl) {
          var m = String((pl && pl.shader_mode) || "");
          if (m === "noderiv") return "noderiv";
          if (m === "rint") return "rint";
          if (m === "fma_pair" || m === "fma") return "fma";
          if (m === "denorm" || m === "ftz") return "denorm";
          if (m === "ulp" || m === "silicon_ulp") return "ulp";
          if (!m || m === "float") return "float";
          return "other";
        };
        var picked = [];
        // iss/67: denorm before rint so Lane-S FTZ seats under cap≥4 as well as cap=5
        var roleOrder = ["noderiv", "float", "fma", "denorm", "rint", "ulp", "other"];
        var ri;
        for (ri = 0; ri < roleOrder.length && picked.length < PATH_CAP; ri++) {
          var want = roleOrder[ri];
          var pi;
          for (pi = 0; pi < plans.length && picked.length < PATH_CAP; pi++) {
            if (roleOf(plans[pi]) === want && picked.indexOf(plans[pi]) < 0) {
              picked.push(plans[pi]);
            }
          }
        }
        plans = picked.length ? picked : plans.slice(0, PATH_CAP);
      } else {
        plans = plans.slice(0, PATH_CAP);
      }
    }

    function scorePaths(paths) {
      var okPaths = paths.filter(function (p) {
        return p && p.ok && p.curve && p.entropy_ok;
      });
      var keyCount = {};
      var means = [];
      var meanBucket = {};
      okPaths.forEach(function (p) {
        var k = p.magrank_key || "";
        if (k) keyCount[k] = (keyCount[k] || 0) + 1;
        if (p.mean != null && isFinite(p.mean)) {
          means.push(p.mean);
          var b = String(Math.round(p.mean * 1000));
          if (!meanBucket[b]) meanBucket[b] = { n: 0, mean: p.mean };
          meanBucket[b].n += 1;
        }
      });
      // Dominant 0.001 mean cluster (not even-median which can land on noderiv 0.261).
      var medianMean = null;
      var bestBn = -1;
      var bestBk = 1e18;
      Object.keys(meanBucket).forEach(function (b) {
        var n = meanBucket[b].n;
        var bk = Number(b);
        if (n > bestBn || (n === bestBn && bk < bestBk)) {
          bestBn = n;
          bestBk = bk;
          medianMean = meanBucket[b].mean;
        }
      });
      var best = null;
      var bestScore = -1e9;
      okPaths.forEach(function (p) {
        var score = 0;
        score += (keyCount[p.magrank_key] || 0) * 10;
        if (p.size === 128) score += 3;
        if (p.warm_frames >= 4) score += 2;
        else if (p.warm_frames >= 2) score += 1;
        if (p.std != null && p.std > 0.02) score += 1;
        if (p.mean != null && p.mean > 0.15 && p.mean < 0.4) score += 1;
        // Prefer mean near median of session paths (stable cluster, not cold outlier).
        if (medianMean != null && p.mean != null) {
          var d = Math.abs(p.mean - medianMean);
          if (d < 0.0005) score += 4;
          else if (d < 0.001) score += 2;
          else if (d > 0.002) score -= 2;
        }
        // Prefer noderiv/hard paths for cross-engine commercial floor alignment (Lane-C).
        // Mirror server trust.rs weights so FE provisional select does not stick on warm4.
        if (/noderiv_hard/.test(p.path_id) || p.shader_mode === "noderiv") score += 8;
        else if (/rint_/.test(p.path_id) || p.shader_mode === "rint") score += 7;
        else if (/noderiv/.test(p.path_id)) score += 6;
        else if (/_std$|warm4_v3f_128$|unk_/.test(p.path_id)) score += 0.5;
        p.select_score = score;
        if (score > bestScore) {
          bestScore = score;
          best = p;
        }
      });
      if (!best) {
        var anyOk = paths.filter(function (p) {
          return p && p.ok && p.curve;
        });
        if (anyOk.length) best = anyOk[0];
      }
      var pair_agreements = [];
      if (best && best.curve) {
        okPaths.forEach(function (p) {
          if (p.path_id === best.path_id) return;
          var l2 = 0;
          var n = Math.min(p.curve.length, best.curve.length);
          var i;
          for (i = 0; i < n; i++) {
            var dd = (Number(p.curve[i]) || 0) - (Number(best.curve[i]) || 0);
            l2 += dd * dd;
          }
          pair_agreements.push({
            path_id: p.path_id,
            magrank_eq: p.magrank_key === best.magrank_key,
            l2: Math.round(Math.sqrt(l2 / (n || 1)) * 1e6) / 1e6,
            mean_delta:
              p.mean != null && best.mean != null
                ? Math.round(Math.abs(p.mean - best.mean) * 1e9) / 1e9
                : null,
          });
        });
      }
      // Ops summary once per page (low traffic) — residual/curve scalars only, no raw series.
      try {
        if (
          global.GROps &&
          typeof GROps.report === "function" &&
          !global.__GR_MP_OPS_REPORTED__
        ) {
          global.__GR_MP_OPS_REPORTED__ = 1;
          var modeTags = [];
          okPaths.forEach(function (p) {
            var m = p.shader_mode || (/noderiv/.test(p.path_id) ? "noderiv" : /rint/.test(p.path_id) ? "rint" : /ulp/.test(p.path_id) ? "ulp" : "float");
            if (modeTags.indexOf(m) < 0) modeTags.push(m);
          });
          GROps.report(
            "multipath_done",
            "b10",
            {
              n_paths: paths.length,
              n_ok: okPaths.length,
              path_cap: PATH_CAP,
              weak: !!weak,
              profile: profile,
              engine: engine,
              residual_mean: best != null ? best.mean : null,
              residual_std: best != null ? best.std : null,
              chosen_path_id: best ? best.path_id : null,
              entropy_ok: best ? !!best.entropy_ok : false,
              modes: modeTags.slice(0, 6),
            },
            "info"
          );
        }
      } catch (eOpsMp) {}
      // Multi-path fused materials (commercial entropy): do not discard sibling paths.
      // Concat path means + stds + first-8 of each ok curve so res/wg can separate
      // same-mean / different-mode sessions (prod class-floor fix).
      var fusedCurve = null;
      var pathMeans = [];
      var pathStds = [];
      var pathModes = [];
      try {
        if (okPaths.length >= 1) {
          fusedCurve = [];
          okPaths.forEach(function (p) {
            if (p.mean != null && isFinite(p.mean)) {
              pathMeans.push(Math.round(Number(p.mean) * 1e9) / 1e9);
            }
            if (p.std != null && isFinite(p.std)) {
              pathStds.push(Math.round(Number(p.std) * 1e9) / 1e9);
            }
            var mode =
              p.shader_mode ||
              (/noderiv/.test(p.path_id || "")
                ? "noderiv"
                : /rint/.test(p.path_id || "")
                  ? "rint"
                  : /ulp/.test(p.path_id || "")
                    ? "ulp"
                    : "float");
            pathModes.push(mode);
            if (p.curve && p.curve.length) {
              var take = Math.min(8, p.curve.length);
              var ci;
              for (ci = 0; ci < take; ci++) {
                fusedCurve.push(Number(p.curve[ci]) || 0);
              }
            }
          });
          // Prefer primary full curve when multipath fused is short
          if (best && best.curve && best.curve.length >= 16) {
            if (!fusedCurve.length || fusedCurve.length < best.curve.length) {
              // keep best as primary curve; fused is separate material
            }
          }
          if (fusedCurve.length > 64) fusedCurve = fusedCurve.slice(0, 64);
        }
      } catch (eFuse) {
        fusedCurve = null;
      }
      return {
        curve: best && best.curve ? best.curve : null,
        // Longer multipath signature for wg/res entropy (server may prefer)
        webgl_residual_multipath: fusedCurve,
        residual_path_means: pathMeans,
        residual_path_stds: pathStds,
        residual_path_modes: pathModes,
        residual_algo:
          (best && best.residual_algo) || "gr_webgl_residual_std_v3f_multipath",
        residual_probe_engine: engine,
        residual_probe_profile: probeProfileForEngine(engine),
        residual_mean: best ? best.mean : null,
        residual_std: best ? best.std : null,
        residual_paths: paths,
        residual_select: {
          policy: "ensemble_nearest_v1_fe_provisional",
          chosen_path_id: best ? best.path_id : null,
          score: best ? best.select_score : null,
          median_mean: medianMean,
          pair_agreements: pair_agreements,
          weak_device: weak,
          n_paths: paths.length,
          n_ok: okPaths.length,
          path_cap: PATH_CAP,
          multipath_profile: profile,
          note: "async multipath; server re-selects; commercial digests keep 0.001 precision",
        },
        multipath_profile: profile,
      };
    }

    var paths = [];
    var pi = 0;
    function runNext() {
      if (pi >= plans.length) {
        return Promise.resolve(scorePaths(paths));
      }
      var plan = plans[pi++];
      var row;
      try {
        row = webglResidualCurveV3fRun(plan);
      } catch (eRun) {
        row = {
          path_id: plan.path_id,
          ok: false,
          err: String(eRun && eRun.message ? eRun.message : eRun),
        };
      }
      if (row && row.ok && row.curve) {
        row.entropy_ok = residualPathEntropyOk(row.curve);
        row.magrank_key = residualMagrankKey(row.curve);
        if (!row.shader_mode && plan.shader_mode) row.shader_mode = plan.shader_mode;
      } else if (row) {
        row.entropy_ok = false;
        row.magrank_key = "";
      }
      paths.push(row);
      // Soft-release after each path (shared pool reuse; avoid Firefox loseContext spam).
      try {
        if (global.GRGlGovernor) {
          if (GRGlGovernor.releaseSoft) GRGlGovernor.releaseSoft();
          else if (GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
        } else releaseWebglProbeContexts();
      } catch (eRelPath) {}
      // Yield + post-release cooldown. Advanced fma/denorm shaders need longer cool-down
      // so multipath does not thrash GPU driver / freeze weak tabs (browser pressure control).
      var pathGapMs = 55;
      try {
        var sm = String((plan && plan.shader_mode) || "");
        if (sm === "fma_pair" || sm === "fma" || sm === "denorm" || sm === "ftz" || sm === "ulp") {
          pathGapMs = weak ? 110 : 80;
        } else if (weak) {
          pathGapMs = 75;
        }
      } catch (eGap) {}
      return yieldProbeGap().then(function () {
        return new Promise(function (resolve) {
          setTimeout(function () {
            resolve(runNext());
          }, pathGapMs);
        });
      });
    }
    return runNext().then(function (scored) {
      try {
        if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
        else releaseWebglProbeContexts();
      } catch (eRel) {}
      return scored;
    });
  }

  /** Multi-path ensemble (Promise). Callers must then(). */
  function webglResidualCurveV3e(profile) {
    return webglResidualMultiPath(profile ? { profile: profile } : {})
      .then(function (mp) {
        if (!mp || !mp.curve || mp.curve.length < 8) return null;
        return {
          curve: mp.curve,
          residual_algo: mp.residual_algo,
          residual_probe_engine: mp.residual_probe_engine,
          residual_probe_profile: mp.residual_probe_profile,
          residual_mean: mp.residual_mean,
          residual_std: mp.residual_std,
          residual_paths: mp.residual_paths,
          residual_select: mp.residual_select,
          multipath_profile: mp.multipath_profile || profile || "default",
        };
      })
      .catch(function () {
        return null;
      });
  }

  /**
   * Server stack_auth consumes residual_mean / residual_hist.
   * @param {number} [seedMix] optional constant mixed into FS (multi-seed unit surface)
   */
  function webglResidualMean(seedMix) {
    try {
      var c = document.createElement("canvas");
      c.width = 256;
      c.height = 256;
      var gl =
        c.getContext("webgl", { antialias: false, preserveDrawingBuffer: true }) ||
        c.getContext("experimental-webgl", { antialias: false, preserveDrawingBuffer: true });
      if (!gl) return { residual_mean: null, residual_available: false, residual_hist: null };
      var hasDeriv = false;
      try {
        hasDeriv = !!gl.getExtension("OES_standard_derivatives");
      } catch (eExt) {
        hasDeriv = false;
      }
      var linked = linkWebglProgram(
        gl,
        "attribute vec2 a; void main(){ gl_Position=vec4(a,0.0,1.0); }",
        residualFsCandidates(seedMix, hasDeriv, 256)
      );
      if (!linked) {
        return { residual_mean: null, residual_available: false, residual_hist: null };
      }
      var prog = linked.prog;
      var buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
      gl.useProgram(prog);
      var loc = gl.getAttribLocation(prog, "a");
      if (loc < 0) {
        return { residual_mean: null, residual_available: false, residual_hist: null };
      }
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
      gl.viewport(0, 0, 256, 256);
      gl.clearColor(0, 0, 0, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      var px = new Uint8Array(256 * 256 * 4);
      gl.readPixels(0, 0, 256, 256, gl.RGBA, gl.UNSIGNED_BYTE, px);
      var sum = 0;
      var n = 0;
      var bins = 32;
      var hist = new Array(bins);
      var bi;
      for (bi = 0; bi < bins; bi++) hist[bi] = 0;
      var i;
      for (i = 0; i < px.length; i += 16) {
        var v = px[i] / 255;
        sum += v;
        n++;
        var b = Math.min(bins - 1, Math.floor(v * bins));
        hist[b] += 1;
      }
      var mean = n ? sum / n : null;
      if (n > 0) {
        for (bi = 0; bi < bins; bi++) hist[bi] = Math.round((hist[bi] / n) * 1e6) / 1e6;
      }
      var acc = "";
      for (i = 0; i < px.length; i += 17 * 4) acc += px[i] + ",";
      return {
        residual_mean: mean != null ? Math.round(mean * 1e12) / 1e12 : null,
        residual_hist: n ? hist : null,
        residual_available: mean != null,
        residual_algo: linked.algo || "gr_webgl_residual_hist_v3b",
        surface_digest: n ? fnv1a(acc) : null,
        hist_digest: n ? fnv1a(hist.join(",")) : null,
      };
    } catch (e) {
      return {
        residual_mean: null,
        residual_available: false,
        residual_hist: null,
        residual_error: String(e && e.message ? e.message : e),
      };
    }
  }

  /**
   * Mobile / constrained form-factor — same probe richness, longer yields between chunks.
   * Does not skip fields or reduce ops; only spaces high-load work.
   */
  function probeIsMobileLike() {
    try {
      var ua = String((navigator && navigator.userAgent) || "");
      if (/Mobi|Android|iPhone|iPad|iPod|Mobile|webOS|BlackBerry|IEMobile|Opera Mini/i.test(ua)) {
        return true;
      }
      var w = (screen && screen.width) || (typeof innerWidth !== "undefined" ? innerWidth : 0);
      var tp = (navigator && navigator.maxTouchPoints) || 0;
      if (tp > 1 && w > 0 && w < 900) return true;
      if (navigator && navigator.userAgentData && navigator.userAgentData.mobile) return true;
    } catch (eM) {}
    return false;
  }

  /**
   * Very weak device (low RAM/cores/save-data) — still full ops; only longer yields.
   * Subset of mobile but also low-end desktop / tablet.
   */
  function probeIsWeakDevice() {
    try {
      if (probeIsMobileLike()) {
        // Mobile + low memory/cores → weak
        var dm = navigator.deviceMemory;
        var hc = navigator.hardwareConcurrency;
        if (dm != null && dm <= 4) return true;
        if (hc != null && hc <= 4) return true;
        // All phones get at least mobile yield; "weak" is stricter for extra gap
        if (dm != null && dm <= 2) return true;
        if (hc != null && hc <= 2) return true;
      }
      if (navigator.deviceMemory != null && navigator.deviceMemory <= 2) return true;
      if (navigator.hardwareConcurrency != null && navigator.hardwareConcurrency <= 2) return true;
      var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      if (conn && (conn.saveData || conn.effectiveType === "slow-2g" || conn.effectiveType === "2g")) {
        return true;
      }
    } catch (eW) {}
    return false;
  }

  /** Yield between high-load probe chunks. Weak/mobile get longer gaps — never fewer ops. */
  function yieldProbeGap() {
    var ms = 4;
    try {
      if (probeIsWeakDevice()) ms = 40;
      else if (probeIsMobileLike()) ms = 20;
      else if (typeof document !== "undefined" && document.hidden) ms = 12;
      // Lab pressure flag: stretch yields to protect browser responsiveness.
      if (global.__GR_LAB_PRESSURE__) ms = Math.max(ms, 48);
    } catch (eY) {}
    return new Promise(function (resolve) {
      setTimeout(resolve, ms);
    });
  }

  /**
   * Demo D41 compact → production: multi-seed residual surface (N=8 + 3 identical-draw).
   * Full richness preserved. Returns a Promise (async yields) — callers must await/then.
   * First seed residual_hist kept for stack_auth. HW mutex prevents concurrent GPU packs.
   */
  function webglUnitSurfaceCompact() {
    return webglUnitSurfaceCompactAsync();
  }

  function webglUnitSurfaceCompactAsync() {
    var n = 8;
    var joints = [];
    var means = [];
    var firstHist = null;
    var si = 0;
    var CHUNK = probeIsMobileLike() ? 1 : 2;

    function oneSeed(ix) {
      var r = webglResidualMean(ix * 0.137 + 0.013);
      if (!r || r.residual_mean == null) {
        return { fail: true, multi_seed_n: ix };
      }
      if (ix === 0 && r.residual_hist && r.residual_hist.length) {
        firstHist = r.residual_hist;
      }
      joints.push(
        fnv1a(
          String(r.hist_digest || "") +
            "|" +
            Number(r.residual_mean).toFixed(4) +
            "|" +
            String(r.surface_digest || "")
        )
      );
      means.push(Number(r.residual_mean).toFixed(4));
      return { fail: false };
    }

    function seedLoop() {
      var end = Math.min(si + CHUNK, n);
      for (; si < end; si++) {
        var st = oneSeed(si);
        if (st.fail) {
          return Promise.resolve({
            unit_surface_available: false,
            multi_seed_n: st.multi_seed_n,
          });
        }
      }
      if (si < n) {
        return yieldProbeGap().then(seedLoop);
      }
      return Promise.resolve(null);
    }

    return seedLoop()
      .then(function (early) {
        if (early) return early;
        return yieldProbeGap().then(function () {
          // Second pass identical-draw noise (3 draws, seed 0) — full multiround check
          var idDigs = [];
          var j;
          for (j = 0; j < 3; j++) {
            var r2 = webglResidualMean(0);
            if (r2 && r2.surface_digest) idDigs.push(r2.surface_digest);
          }
          var uniqueId = {};
          var uq = 0;
          for (j = 0; j < idDigs.length; j++) {
            if (!uniqueId[idDigs[j]]) {
              uniqueId[idDigs[j]] = 1;
              uq++;
            }
          }
          var surface_seq = fnv1a(joints.join("|"));
          var mean_seq = fnv1a(means.join(","));
          return {
            unit_surface_available: true,
            multi_seed_n: n,
            unit_surface_id: fnv1a(surface_seq + "|" + mean_seq),
            unit_surface_algo: "gr_unit_v1",
            unit_multiround_stable: uq <= 1,
            identical_draw_unique: uq,
            residual_hist: firstHist,
          };
        });
      })
      .catch(function (eU) {
        return {
          unit_surface_available: false,
          unit_surface_error: String(eU && eU.message ? eU.message : eU),
        };
      });
  }

  /** Soft-GL label heuristic (claim layer only — not stack_class). */
  function softwareRendererHeuristic(renderer) {
    return /swiftshader|llvmpipe|softpipe|microsoft basic render|software|virtio|vmware|virtualbox|subzero/i.test(
      String(renderer || "")
    );
  }

  function rendererClassFromLabel(renderer) {
    var s = String(renderer || "").toLowerCase();
    if (!s) return "unknown";
    if (s.indexOf("swiftshader") >= 0 || s.indexOf("subzero") >= 0) return "swiftshader";
    if (s.indexOf("llvmpipe") >= 0 || s.indexOf("softpipe") >= 0) return "llvmpipe";
    if (s.indexOf("virtio") >= 0 || s.indexOf("vmware") >= 0) return "virt_gpu";
    if (s.indexOf("nvidia") >= 0 || s.indexOf("geforce") >= 0) return "angle_nvidia";
    if (s.indexOf("amd") >= 0 || s.indexOf("radeon") >= 0) return "angle_amd";
    if (s.indexOf("intel") >= 0) return "angle_intel";
    if (s.indexOf("apple") >= 0 || s.indexOf("metal") >= 0 || s.indexOf("m1") >= 0) return "apple_gpu";
    if (s.indexOf("adreno") >= 0 || s.indexOf("mali") >= 0) return "mobile_gpu";
    return "other";
  }

  /**
   * FE field helper for residual/stack signals — **not** commercial authority.
   * Does NOT embed lab residual means or digest-hex oracles (iss/15 P0-2).
   * Soft label heuristic only; residual_mean/hist uploaded for server stack_auth.
   * stack_class stays unknown unless soft label is obvious (server re-classifies).
   */
  function envStackFusion(fields) {
    fields = fields || {};
    var mean = fields.residual_mean != null ? Number(fields.residual_mean) : null;
    var renderer = fields.webgl_unmasked_renderer || fields.webgl_renderer || "";
    var rc = rendererClassFromLabel(renderer);
    var softLabel = softwareRendererHeuristic(renderer);
    var highEnd =
      rc === "angle_nvidia" ||
      rc === "angle_amd" ||
      rc === "apple_gpu" ||
      rc === "angle_intel" ||
      rc === "mobile_gpu";

    // Only claim soft_render from honest soft labels; residual classification is server-side.
    var residual_soft_like = softLabel ? true : null;
    var stack_class = softLabel ? "soft_render" : "unknown";

    var reasons = [];
    var spoof_score = 0;
    if (highEnd && softLabel) {
      reasons.push("webgl_label_vs_soft");
      spoof_score += 0.25;
    }
    if (softLabel) reasons.push("soft_gl");
    // Caps claim vs soft label (not residual oracle)
    if (
      softLabel &&
      fields.webgl_max_texture != null &&
      Number(fields.webgl_max_texture) >= 16384 &&
      highEnd
    ) {
      reasons.push("caps_claim_vs_soft_behavior");
      spoof_score += 0.15;
    }
    if (spoof_score > 1) spoof_score = 1;

    var vm_score = 0;
    if (stack_class === "soft_render") vm_score += 0.25;
    if (rc === "virt_gpu") vm_score += 0.35;
    if (vm_score > 1) vm_score = 1;

    var out = {
      residual_soft_like: residual_soft_like,
      residual_mean: mean != null && !isNaN(mean) ? mean : null,
      renderer_class: rc,
      stack_class: stack_class,
      spoof_score: Math.round(spoof_score * 1000) / 1000,
      vm_score: Math.round(vm_score * 1000) / 1000,
      env_reasons: reasons,
      software_renderer_heuristic: softLabel,
      claims_host_silicon_recovery: false,
      env_fusion_algo: "gr_env_fusion_v2_fields_only",
      authenticity_hint:
        spoof_score >= 0.4
          ? "fp_spoof_suspect"
          : stack_class === "soft_render"
            ? "soft_stack"
            : "unknown",
    };
    if (fields.residual_hist && fields.residual_hist.length) {
      out.residual_hist = fields.residual_hist;
    }
    return out;
  }

  /** Canvas / WebGL structural params (generic APIs) — surface materials, not engine names. */
  function surfaceMaterials() {
    var out = {};
    try {
      var c = document.createElement("canvas");
      c.width = 64;
      c.height = 32;
      var ctx = c.getContext("2d");
      if (ctx) {
        ctx.textBaseline = "top";
        ctx.font = "14px Arial";
        ctx.fillStyle = "#f60";
        ctx.fillRect(0, 0, 64, 32);
        ctx.fillStyle = "#069";
        ctx.fillText("gr", 2, 2);
        out.canvas_2d_hash = simpleHash(c.toDataURL());
      }
      var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
      if (gl) {
        out.webgl_max_texture = gl.getParameter(gl.MAX_TEXTURE_SIZE) || null;
        out.webgl_max_renderbuffer = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) || null;
        out.webgl_max_viewport = (function () {
          try {
            var v = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
            return v ? [v[0], v[1]] : null;
          } catch (e) {
            return null;
          }
        })();
        out.webgl_vendor = gl.getParameter(gl.VENDOR) || "";
        out.webgl_renderer = gl.getParameter(gl.RENDERER) || "";
        out.webgl_version = gl.getParameter(gl.VERSION) || "";
        var dbg = gl.getExtension("WEBGL_debug_renderer_info");
        if (dbg) {
          out.webgl_unmasked_renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "";
          out.webgl_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) || "";
        }
        // Structural param fingerprint (extensions sorted) — stable-ish on same GPU path
        try {
          var exts = gl.getSupportedExtensions() || [];
          out.webgl_ext_count = exts.length;
          out.webgl_ext_hash = simpleHash(exts.slice().sort().join(","));
        } catch (e4) {}
        out.software_renderer_heuristic = softwareRendererHeuristic(
          out.webgl_unmasked_renderer || out.webgl_renderer
        );
        out.renderer_class = rendererClassFromLabel(
          out.webgl_unmasked_renderer || out.webgl_renderer
        );
      }
    } catch (e) {}
    // Cheap residual mean + hist (static short-visit path) — server classifies stack
    try {
      var res = webglResidualMean();
      if (res) {
        if (res.residual_mean != null) out.residual_mean = res.residual_mean;
        if (res.residual_hist) out.residual_hist = res.residual_hist;
        out.residual_available = !!res.residual_available;
        if (res.residual_algo) out.residual_algo = res.residual_algo;
      }
      var fusion = envStackFusion(out);
      Object.keys(fusion).forEach(function (k) {
        out[k] = fusion[k];
      });
    } catch (eR) {}
    return out;
  }

  function fieldsBootstrap() {
    var nav = navigator || {};
    var scr = screen || {};
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    var ms = machineStableSignals();
    var f = {
      user_agent: ua,
      language: nav.language || "",
      languages: nav.languages ? Array.prototype.slice.call(nav.languages) : [],
      platform: platform,
      vendor: nav.vendor || "",
      product_sub: nav.productSub || "",
      app_version: nav.appVersion || "",
      os_family: deriveOsFamily(ua, platform),
      hardware_concurrency: ms.hardware_concurrency,
      device_memory: ms.device_memory,
      screen_width: scr.width || null,
      screen_height: scr.height || null,
      screen_avail_width: ms.screen_avail_width,
      screen_avail_height: ms.screen_avail_height,
      timezone: ms.timezone || "",
      timezone_offset_min: (function () {
        try {
          return new Date().getTimezoneOffset();
        } catch (e) {
          return null;
        }
      })(),
      cookie_enabled: ms.cookie_enabled,
      max_touch_points: ms.max_touch_points,
      color_depth: ms.color_depth,
      pixel_depth: ms.pixel_depth,
      device_pixel_ratio: ms.device_pixel_ratio,
      audio_sample_rate: ms.audio_sample_rate,
      audio_base_latency: ms.audio_base_latency != null ? ms.audio_base_latency : null,
      intl_locale: ms.intl_locale || "",
      intl_calendar: ms.intl_calendar || "",
      pdf_viewer: ms.pdf_viewer,
      plugins_length: ms.plugins_length,
      do_not_track: nav.doNotTrack != null ? String(nav.doNotTrack) : null,
      outer_width: typeof window !== "undefined" ? window.outerWidth : null,
      outer_height: typeof window !== "undefined" ? window.outerHeight : null,
      inner_width: typeof window !== "undefined" ? window.innerWidth : null,
      inner_height: typeof window !== "undefined" ? window.innerHeight : null,
      net_effective_type: ms.net_effective_type || "",
      net_downlink: ms.net_downlink,
      net_rtt: ms.net_rtt,
      ua_platform: ms.ua_platform || "",
      form_class: (function () {
        var w = scr.width || 0;
        var touch = nav.maxTouchPoints || 0;
        if (w > 0 && w < 600) return "mobile";
        if (touch > 1 && w > 0 && w < 900) return "mobile";
        return "desktop";
      })(),
    };
    // Earliest GL caps (OSS FPJS-style sync getParameter) so short bounce /
    // pagehide before B2/B10 still carries texture for class:t* Model-ID.
    try {
      var caps0 = webglCapsLite();
      if (caps0 && typeof caps0 === "object") {
        Object.keys(caps0).forEach(function (ck) {
          if (f[ck] == null && caps0[ck] != null && caps0[ck] !== "") {
            f[ck] = caps0[ck];
          }
        });
        f.b0_caps_coland = true;
      }
    } catch (eCaps0) {}
    return f;
  }

  /** Common identity surface for multi-source soft compare (main/iframe/worker). */
  function identitySurfaceFields() {
    var nav = navigator || {};
    var ua = nav.userAgent || "";
    var platform = nav.platform || "";
    return {
      user_agent: ua,
      platform: platform,
      os_family: deriveOsFamily(ua, platform),
      language: nav.language || "",
      languages: nav.languages ? Array.prototype.slice.call(nav.languages) : [],
      vendor: nav.vendor || "",
      hardware_concurrency: nav.hardwareConcurrency != null ? nav.hardwareConcurrency : null,
      device_memory: nav.deviceMemory != null ? nav.deviceMemory : null,
      max_touch_points: nav.maxTouchPoints != null ? nav.maxTouchPoints : null,
      webdriver: !!nav.webdriver,
      timezone: (function () {
        try {
          return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
        } catch (e) {
          return "";
        }
      })(),
      timezone_offset_min: (function () {
        try {
          return new Date().getTimezoneOffset();
        } catch (e2) {
          return null;
        }
      })(),
    };
  }

  /** Font presence sample (analysis density without full 400-font matrix). */
  function fontPresenceSample() {
    var probes = [
      "Arial",
      "Helvetica",
      "Times New Roman",
      "Courier New",
      "Georgia",
      "Verdana",
      "Comic Sans MS",
      "Impact",
      "Trebuchet MS",
      "Palatino Linotype",
      "Lucida Console",
      "Tahoma",
      "Segoe UI",
      "Roboto",
      "Noto Sans",
      "Microsoft YaHei",
      "PingFang SC",
      "Hiragino Sans",
      "Menlo",
      "Monaco",
    ];
    var present = [];
    try {
      var base = "monospace";
      var canvas = document.createElement("canvas");
      var ctx2 = canvas.getContext("2d");
      if (!ctx2) return { font_count: 0, fonts_present: [] };
      function w(font) {
        ctx2.font = "16px " + font + "," + base;
        return ctx2.measureText("mmmmmmmmmmlli").width;
      }
      var baseW = w(base);
      probes.forEach(function (f) {
        try {
          if (Math.abs(w(f) - baseW) > 0.5) present.push(f);
        } catch (e) {}
      });
    } catch (e2) {}
    return { font_count: present.length, fonts_present: present.slice(0, 24) };
  }

  /** WebGL1/2 caps + extension list hash (not full enum dump). */
  function webglCapsLite() {
    var out = { webgl_support: false, webgl2_support: false };
    function applyTexture(gl) {
      if (!gl) return;
      try {
        var tex = gl.getParameter(gl.MAX_TEXTURE_SIZE);
        if (typeof tex === "number" && isFinite(tex) && tex > 0) {
          out.gl_max_texture_size = tex | 0;
          out.webgl_max_texture = tex | 0;
        }
      } catch (eT) {}
    }
    try {
      var c = document.createElement("canvas");
      var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
      if (gl) {
        out.webgl_support = true;
        var ext = gl.getExtension("WEBGL_debug_renderer_info");
        if (ext) {
          out.webgl_unmasked_renderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) || "";
          out.webgl_unmasked_vendor = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL) || "";
        }
        out.webgl_vendor = gl.getParameter(gl.VENDOR) || "";
        out.webgl_renderer = gl.getParameter(gl.RENDERER) || "";
        out.webgl_version = gl.getParameter(gl.VERSION) || "";
        applyTexture(gl);
        var exts = gl.getSupportedExtensions() || [];
        out.webgl_extensions_count = exts.length;
        out.webgl_extensions_hash = simpleHash(exts.slice().sort().join("|")).slice(0, 16);
      }
      var gl2 = c.getContext("webgl2");
      out.webgl2_support = !!gl2;
      if (gl2) {
        out.webgl2_version = gl2.getParameter(gl2.VERSION) || "";
        // WebGL1 path may omit texture under governor/farbling — retry on GL2.
        if (out.gl_max_texture_size == null) applyTexture(gl2);
      }
    } catch (e) {}
    return out;
  }

  /** Canvas 2d short hash (soft signal only — not commercial digest). */
  function canvasHashLite() {
    try {
      var c = document.createElement("canvas");
      c.width = 64;
      c.height = 24;
      var ctx2 = c.getContext("2d");
      if (!ctx2) return null;
      ctx2.textBaseline = "top";
      ctx2.font = "14px Arial";
      ctx2.fillStyle = "#f60";
      ctx2.fillRect(0, 0, 64, 24);
      ctx2.fillStyle = "#069";
      ctx2.fillText("gr", 2, 2);
      var data = ctx2.getImageData(0, 0, 64, 24).data;
      var acc = 0;
      for (var i = 0; i < data.length; i += 17) acc = (acc * 33 + data[i]) >>> 0;
      return "c_" + acc.toString(16);
    } catch (e) {
      return null;
    }
  }

  /** Math FP digest (D25 lite). */
  function mathDigestLite() {
    try {
      var vals = [
        Math.tan(-1e300),
        Math.sin(Math.PI / 2),
        Math.cos(1e-12),
        Math.exp(1),
        Math.log(Math.E),
        Math.sqrt(2),
        Math.acos(0.5),
        Math.atan2(1, 0),
      ];
      return "m_" + simpleHash(vals.map(function (v) {
        return String(v);
      }).join("|")).slice(0, 16);
    } catch (e) {
      return null;
    }
  }

  /**
   * F-3 / iss18 §7.4: default-strip raw samples & bulky arrays (bandwidth + not analyzed as material).
   * Lab override: window.__GR_UPLOAD_SAMPLES__ = true  or  GR_UPLOAD_SAMPLES=1 (node).
   */
  var SAMPLE_STRIP_KEYS = {
    audio_deep_curve: 1,
    audio_deep_sr: 1,
    css_props_sample: 1,
    gamepad_ids_sample: 1,
    h01_points: 1,
    challenge_pixel_sample: 1,
    media_query_true_sample: 1,
    native_integrity_sample: 1,
    speech_voices_sample: 1,
    layer_divergence_sample: 1,
    font_present_sample: 1,
    gpu_query_meta: 1,
    caps_probe_results: 1,
    webrtc_host_ips: 1,
    sandbox_sources: 1,
    object_census_leaf_est: 1,
    math_acc: 1,
    wasm_acc: 1,
    raf_samples: 1,
    gpu_ns_staircase: 1, // keep gpu_ns_staircase_digest + points/median
    gpu_wall_staircase: 1, // keep digests / slope / r2
    gpu_bandwidth_ladder: 1,
  };

  function shouldUploadSamples() {
    try {
      if (typeof window !== "undefined" && window.__GR_UPLOAD_SAMPLES__) return true;
    } catch (e0) {}
    try {
      if (typeof process !== "undefined" && process.env && process.env.GR_UPLOAD_SAMPLES === "1")
        return true;
    } catch (e1) {}
    return false;
  }

  function stripRawSamples(fields) {
    if (!fields || typeof fields !== "object") return fields;
    if (shouldUploadSamples()) return fields;
    var out = fields;
    var copied = false;
    function drop(k) {
      if (out[k] === undefined) return;
      if (!copied) {
        out = Object.assign({}, out);
        copied = true;
      }
      delete out[k];
    }
    Object.keys(SAMPLE_STRIP_KEYS).forEach(drop);
    // Generic *_sample / *_samples
    Object.keys(out).forEach(function (k) {
      if (/_samples?$/.test(k) && k !== "device_motion_sample_ok") drop(k);
    });
    return out;
  }

  /**
   * Matrix-driven sync family enrich for every B-batch (probe_fallback_priority).
   * Only fills **missing** keys; never overwrites successful primary path values.
   * Async silicon (residual/audio) is filled on B10/B17/nest; here we cover
   * nav/display/network/cores that all batches may carry for multi-source mint.
   */
  var BATCH_FALLBACK_FAMILIES = {
    B0_bootstrap: ["ua_platform", "hardware_concurrency", "display"],
    B1_conflict: ["ua_platform", "hardware_concurrency"],
    B2_hardware: ["hardware_concurrency", "display", "webgl_residual"],
    B3_system: ["ua_platform", "hardware_concurrency", "display"],
    B4_mobile: ["ua_platform", "display", "hardware_concurrency"],
    B5_census: ["ua_platform", "display", "network_class"],
    B6_risk: ["ua_platform", "hardware_concurrency"],
    B7_sandbox: ["ua_platform", "hardware_concurrency"],
    B8_gateway_early: ["network_class"],
    B9_network: ["network_class", "webrtc_host"],
    B10_hw_curves: [
      "webgl_residual",
      "audio_noise",
      "canvas_noise",
      "webrtc_host",
      "media_devices",
      "display",
      "network_class",
      "cpu_timing",
      "hardware_concurrency",
    ],
    B11_interaction: ["ua_platform"],
    B12_anti_camouflage: ["ua_platform", "hardware_concurrency"],
    B13_authorized: ["ua_platform"],
    B14_css_protocol: ["display"],
    B15_cross_curves: ["webgl_residual", "canvas_noise", "audio_noise"],
    B16_fast_signals: ["network_class", "audio_noise", "display"],
    B17_hw_physical: [
      "webgl_residual",
      "cpu_timing",
      "display",
      "media_devices",
      "network_class",
      "hardware_concurrency",
    ],
    B18_webgpu: ["webgl_residual"],
    B19_eme_media: ["media_devices"],
    B20_challenge_seed: ["webgl_residual", "canvas_noise"],
    B21_census_volume: ["ua_platform", "display"],
    B22_gpu_timer: ["webgl_residual"],
    B23_native_canvas_hedge: ["canvas_noise"],
    B24_material_crosscheck: ["webgl_residual", "canvas_noise", "audio_noise"],
    B25_clock_raf: ["cpu_timing"],
    B26_agent_parity: ["ua_platform"],
    B27_storage_privacy: ["ua_platform"],
    B28_permissions_media: ["media_devices"],
    B29_sensors_battery: ["hardware_concurrency"],
    B30_gpu_bandwidth: ["webgl_residual"],
    B31_shader_numeric: ["webgl_residual"],
    B33_caps_pressure: ["webgl_residual"],
    B34_cpu_cache_ladder: ["cpu_timing", "hardware_concurrency"],
    B35_dom_perf: ["cpu_timing"],
    B36_raster_msaa: ["webgl_residual", "canvas_noise"],
    B37_thermal_drift_lite: ["cpu_timing"],
    B38_neg_dict: ["ua_platform"],
    B39_mem_pressure: ["hardware_concurrency"],
    B40_websocket_fp: ["network_class"],
    B41_hid_gamepad: ["hardware_concurrency"],
    B42_thermal_drift_full: ["cpu_timing"],
    B43_errors_engine: ["ua_platform"],
    B44_speech_deep: ["ua_platform"],
    B45_display_hdr: ["display"],
    B46_audio_deep: ["audio_noise"],
  };

  function fillMissing(target, src) {
    if (!src) return;
    Object.keys(src).forEach(function (k) {
      if (k === "probe_paths" || k.charAt(0) === "_") return;
      if (target[k] == null || target[k] === "") {
        if (src[k] != null && src[k] !== "") target[k] = src[k];
      }
    });
  }

  /** Sync multipath enrich for a B-batch (matrix families; missing-only). */
  function enrichBatchFallbacksSync(batchId, fields) {
    var f = fields || {};
    var fams = BATCH_FALLBACK_FAMILIES[batchId] || [];
    if (!fams.length && String(batchId || "").charAt(0) === "B") {
      fams = ["ua_platform", "hardware_concurrency"];
    }
    var pathsLog = f.probe_paths && f.probe_paths.slice ? f.probe_paths.slice() : [];
    var eng = detectEngineFamily();
    var form = probeIsMobileLike() ? "mobile" : "desktop";
    fams.forEach(function (fam) {
      try {
        if (fam === "ua_platform") {
          probeWithFallbacks(
            "ua_platform",
            orderPathsForEngine(
              [
                {
                  id: "ua_ch_brands",
                  engines: ["blink"],
                  run: function () {
                    var uad = navigator.userAgentData;
                    if (!uad) return null;
                    return {
                      ua_platform: uad.platform || "",
                      ua_mobile: !!uad.mobile,
                    };
                  },
                },
                {
                  id: "navigator_platform",
                  engines: ["*"],
                  run: function () {
                    return {
                      platform: navigator.platform || "",
                      user_agent: navigator.userAgent || "",
                      os_family: deriveOsFamily(navigator.userAgent || "", navigator.platform || ""),
                    };
                  },
                },
              ],
              eng,
              form
            )
          ).then(function (r) {
            /* sync path uses run sync-only below */
          });
          // Sync immediate paths (no promise dependency for lite stamps)
          try {
            if (f.platform == null) f.platform = navigator.platform || "";
            if (f.user_agent == null) f.user_agent = navigator.userAgent || "";
            if (f.os_family == null)
              f.os_family = deriveOsFamily(f.user_agent || "", f.platform || "");
            pathsLog.push({ name: "ua_platform", ok: !!f.platform, detail: "sync" });
          } catch (eUa) {}
        } else if (fam === "hardware_concurrency") {
          try {
            if (f.hardware_concurrency == null && navigator.hardwareConcurrency != null)
              f.hardware_concurrency = navigator.hardwareConcurrency;
            if (f.device_memory == null && navigator.deviceMemory != null)
              f.device_memory = navigator.deviceMemory;
            pathsLog.push({
              name: "hardware_concurrency",
              ok: f.hardware_concurrency != null,
              detail: "nav_cores",
            });
          } catch (eHw) {}
        } else if (fam === "display") {
          try {
            var scr = window.screen || {};
            if (f.screen_width == null && scr.width != null) f.screen_width = scr.width;
            if (f.screen_height == null && scr.height != null) f.screen_height = scr.height;
            if (f.screen_color_depth == null && scr.colorDepth != null)
              f.screen_color_depth = scr.colorDepth;
            if (f.device_pixel_ratio == null && typeof window.devicePixelRatio === "number")
              f.device_pixel_ratio = window.devicePixelRatio;
            if (f.display_count == null) f.display_count = 1;
            pathsLog.push({ name: "display", ok: f.screen_width != null, detail: "screen_primary" });
          } catch (eD) {}
        } else if (fam === "network_class") {
          try {
            var conn =
              navigator.connection || navigator.mozConnection || navigator.webkitConnection;
            if (conn) {
              if (f.net_effective_type == null) f.net_effective_type = conn.effectiveType || "";
              if (f.net_type == null) f.net_type = conn.type || "";
              if (f.net_downlink == null && typeof conn.downlink === "number")
                f.net_downlink = conn.downlink;
              if (f.net_rtt == null && typeof conn.rtt === "number") f.net_rtt = conn.rtt;
              pathsLog.push({ name: "network_class", ok: true, detail: "network_information" });
            } else {
              pathsLog.push({ name: "network_class", ok: false, detail: "no_api" });
            }
          } catch (eN) {}
        }
        // webgl/audio/canvas/webrtc/media/cpu: primary B10/B17/nest paths; stamp family intent
        else if (
          fam === "webgl_residual" ||
          fam === "audio_noise" ||
          fam === "canvas_noise" ||
          fam === "webrtc_host" ||
          fam === "media_devices" ||
          fam === "cpu_timing"
        ) {
          pathsLog.push({
            name: fam + "_matrix",
            ok: true,
            detail: "primary_or_nest",
          });
        }
      } catch (eFam) {
        pathsLog.push({ name: fam, ok: false, detail: "enrich_err" });
      }
    });
    f.probe_paths = pathsLog;
    f.fallback_matrix_algo = "gr_probe_fallback_priority_v1";
    f.fallback_batch_id = batchId;
    return f;
  }

  function enqueue(ctx, batchId, fields, priority, source) {
    ctx = ctx || {};
    // Defensive: multi-tick/hard packs sometimes lose queue/session on ctx.
    if (!ctx.queue && global.GRUploadQueue) ctx.queue = global.GRUploadQueue;
    if (!ctx.session_id) {
      ctx.session_id =
        global.__GR_SESSION_ID__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.session_id) ||
        "";
    }
    if (!ctx.queue || typeof ctx.queue.enqueue !== "function") {
      try {
        if (global.GROps && GROps.report) {
          GROps.report("enqueue_no_queue", "kick", { batch_id: String(batchId || "") }, "error");
        }
      } catch (eOp) {}
      return;
    }
    var forceMap = ctx.force_recollect_batches;
    var force = !!(forceMap && forceMap[batchId]);
    // B10x must land once (EDH). Do NOT force-reupload after sealed_ok unless
    // brain force_recollect — was causing 10–30× wire storms (iss/65).
    if (String(batchId || "").indexOf("B10x_") === 0 && !force) {
      try {
        var Qchk = ctx.queue || global.GRUploadQueue;
        var sidChk = ctx.session_id || global.__GR_SESSION_ID__ || "";
        var already =
          Qchk &&
          Qchk.alreadySent &&
          sidChk &&
          Qchk.alreadySent({
            session_id: sidChk,
            batch_id: batchId,
            source: source || "main",
          });
        force = !already;
      } catch (eF) {
        force = true;
      }
    }
    var f = stripRawSamples(fields || {});
    // L1 B2 may land first without texture; force one caps upgrade when present.
    if (!force && String(batchId || "") === "B2_hardware") {
      try {
        var hasTex =
          (f.gl_max_texture_size != null && Number(f.gl_max_texture_size) > 0) ||
          (f.webgl_max_texture != null && Number(f.webgl_max_texture) > 0);
        if (hasTex) {
          var Qb2 = ctx.queue || global.GRUploadQueue;
          var sidB2 = ctx.session_id || global.__GR_SESSION_ID__ || "";
          if (
            Qb2 &&
            Qb2.alreadySent &&
            sidB2 &&
            Qb2.alreadySent({
              session_id: sidB2,
              batch_id: "B2_hardware",
              source: source || "main",
            })
          ) {
            force = true;
          }
        }
      } catch (eB2f) {}
    }
    // Matrix-driven multipath fill for all B-batches (missing-only).
    if (String(batchId || "").charAt(0) === "B") {
      try {
        f = enrichBatchFallbacksSync(batchId, f);
      } catch (eEn) {}
    }
    // Stamp FE product_version into every batch payload (lab version filter).
    if (!f.product_version) {
      try {
        var pv =
          global.__GR_PRODUCT_VERSION__ ||
          (global.__GR_BOOT__ &&
            (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
          "";
        if (pv) f = Object.assign({}, f, { product_version: String(pv) });
      } catch (ePv) {}
    }
    // Stamp page_id on every batch so evidence/page_results bind to the view.
    if (f.page_id == null) {
      var pid =
        (ctx && ctx.page_id) ||
        (ctx && ctx.fields && ctx.fields.page_id) ||
        (typeof global !== "undefined" && global.__GR_PAGE_ID__) ||
        null;
      if (pid != null) {
        f = Object.assign({}, f, { page_id: pid });
      }
    }
    try {
      ctx.queue.enqueue({
        session_id: ctx.session_id,
        batch_id: batchId,
        source: source || "main",
        inject_path: ctx.inject_path,
        priority: priority || 50,
        force: force,
        payload: {
          fields: f,
          sandbox_kind: source && String(source).indexOf("worker") === 0 ? "worker" : source && String(source).indexOf("iframe") === 0 ? "iframe" : "main",
        },
      });
    } catch (eEnq) {
      try {
        if (global.GROps && GROps.report) {
          GROps.report(
            "enqueue_throw",
            "kick",
            { batch_id: String(batchId || ""), err: String(eEnq && eEnq.message ? eEnq.message : eEnq) },
            "error"
          );
        }
      } catch (e2) {}
    }
  }

  register("B0_bootstrap", {
    priority: 96,
    schedule: "static",
    batch_id: "B0_bootstrap",
    source: "main",
    layer: "lite5",
    run: function (ctx) {
      enqueue(ctx, "B0_bootstrap", fieldsBootstrap(), 96, "main");
    },
  });

  register("B1_conflict", {
    priority: 95,
    schedule: "static",
    batch_id: "B1_conflict",
    layer: "lite5",
    run: function (ctx) {
      var f = {
        webdriver: !!(navigator.webdriver),
        languages: navigator.languages ? Array.prototype.slice.call(navigator.languages) : [],
        platform: navigator.platform || "",
        user_agent: navigator.userAgent || "",
        automation: {
          webdriver: !!(navigator.webdriver),
          playwright: !!(
            navigator.webdriver && /HeadlessChrome|Playwright/i.test(navigator.userAgent || "")
          ) || !!(window._playwright || window.__playwright || window.__pwInitScripts),
          selenium: !!(
            document &&
            document.documentElement &&
            document.documentElement.getAttribute("webdriver")
          ),
          cdc: false,
          phantom: !!(window.callPhantom || window._phantom),
          nightmare: !!(window.__nightmare),
        },
        // Native integrity lite (D26)
        native_function_toString: (function () {
          try {
            return Function.prototype.toString.call(navigator.permissions && navigator.permissions.query);
          } catch (e) {
            return null;
          }
        })(),
        errors_engine: (function () {
          try {
            null[0];
          } catch (e) {
            return String((e && e.name) || e).slice(0, 64);
          }
          return null;
        })(),
      };
      try {
        f.automation.cdc = !!(
          window.cdc_adoQpoasnfa76pfcZLmcfl_Array || window.cdc_adoQpoasnfa76pfcZLmcfl_Promise
        );
      } catch (e) {}
      try {
        f.chrome_runtime = !!(window.chrome && window.chrome.runtime);
      } catch (e2) {}
      enqueue(ctx, "B1_conflict", f, 95, "main");
    },
  });

  register("B2_hardware", {
    priority: 93,
    schedule: "static",
    batch_id: "B2_hardware",
    layer: "lite5",
    run: function (ctx) {
      var ms = machineStableSignals();
      var glc = webglCapsLite();
      var f = Object.assign(
        {
          hardware_concurrency: navigator.hardwareConcurrency || null,
          device_memory: navigator.deviceMemory || null,
          screen_width: screen.width || null,
          screen_height: screen.height || null,
          screen_avail_width: ms.screen_avail_width,
          screen_avail_height: ms.screen_avail_height,
          max_touch_points: navigator.maxTouchPoints != null ? navigator.maxTouchPoints : null,
          color_depth: screen.colorDepth != null ? screen.colorDepth : null,
          device_pixel_ratio: typeof devicePixelRatio !== "undefined" ? devicePixelRatio : null,
          audio_sample_rate: ms.audio_sample_rate,
          timezone: ms.timezone || "",
          os_family: deriveOsFamily(navigator.userAgent || "", navigator.platform || ""),
          canvas_hash: canvasHashLite(),
          math_digest: mathDigestLite(),
        },
        surfaceMaterials(),
        glc
      );
      // Sync lite first (coverage SLA), yield, then deep async (RTC/media) — avoids stacking on paint.
      enqueue(ctx, "B2_hardware", f, 93, "main");
      return new Promise(function (resolve) {
        setTimeout(resolve, 0);
      }).then(function () {
        return deepMachineProbes().then(function (deep) {
          if (!deep || !Object.keys(deep).length) return null;
          var f2 = Object.assign({}, f, deep, { deep_machine_probes: true });
          enqueue(ctx, "B2_hardware", f2, 94, "main");
          return { batch_id: "B2_hardware", deep: true };
        });
      });
    },
  });

  register("B3_system", {
    priority: 92,
    schedule: "static",
    batch_id: "B3_system",
    layer: "lite5",
    run: function (ctx) {
      var ua = navigator.userAgent || "";
      var platform = navigator.platform || "";
      var fonts = fontPresenceSample();
      var mq = {};
      try {
        if (window.matchMedia) {
          [
            "(prefers-color-scheme: dark)",
            "(prefers-reduced-motion: reduce)",
            "(pointer: coarse)",
            "(hover: hover)",
            "(color-gamut: p3)",
            "(display-mode: standalone)",
          ].forEach(function (q) {
            try {
              mq[q] = !!window.matchMedia(q).matches;
            } catch (e) {}
          });
        }
      } catch (e2) {}
      enqueue(
        ctx,
        "B3_system",
        {
          timezone: (function () {
            try {
              return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
            } catch (e) {
              return "";
            }
          })(),
          timezone_offset_min: new Date().getTimezoneOffset(),
          language: navigator.language || "",
          languages: navigator.languages ? Array.prototype.slice.call(navigator.languages) : [],
          platform: platform,
          os_family: deriveOsFamily(ua, platform),
          hardware_concurrency: navigator.hardwareConcurrency || null,
          device_memory: navigator.deviceMemory || null,
          screen_width: screen.width || null,
          screen_height: screen.height || null,
          color_depth: screen.colorDepth != null ? screen.colorDepth : null,
          device_pixel_ratio: typeof devicePixelRatio !== "undefined" ? devicePixelRatio : null,
          font_count: fonts.font_count,
          fonts_present: fonts.fonts_present,
          media_queries_lite: mq,
          math_digest: mathDigestLite(),
        },
        92,
        "main"
      );
    },
  });

  register("B12_anti_camouflage", {
    priority: 94,
    schedule: "static",
    batch_id: "B12_anti_camouflage",
    layer: "lite5",
    run: function (ctx) {
      var f = {
        webdriver: !!(navigator.webdriver),
        chrome_runtime: !!(window.chrome && window.chrome.runtime),
        // iss/21 T-CDP-1: weak conf hint only (never sole veto). Classic console/debug traps.
        cdp_runtime_hint: (function () {
          try {
            var hits = 0;
            // Error stack often mentions Runtime.evaluate / puppeteer / playwright under CDP
            var st = "";
            try {
              throw new Error("gr_cdp_probe");
            } catch (e0) {
              st = String((e0 && e0.stack) || "");
            }
            if (/puppeteer|playwright|__puppeteer|cdc_|Runtime\.enable/i.test(st)) hits += 1;
            // console.debug toString / getter side channel (naive automation)
            try {
              var cd = console && console.debug;
              if (cd && /native code/i.test(Function.prototype.toString.call(cd)) === false) hits += 1;
            } catch (e1) {}
            // document.$cdc_ / $chrome_asyncScriptInfo leftovers
            try {
              var keys = Object.keys(document);
              for (var i = 0; i < keys.length; i++) {
                if (/^\$cdc_|\$chrome_asyncScriptInfo|__webdriver/i.test(keys[i])) {
                  hits += 1;
                  break;
                }
              }
            } catch (e2) {}
            // iss/38 P1-3: extra weak side-channels (still never sole veto)
            try {
              if (window.domAutomation || window.domAutomationController) hits += 1;
            } catch (e3) {}
            try {
              if (navigator.webdriver === true) hits += 1;
            } catch (e4) {}
            try {
              var desc = Object.getOwnPropertyDescriptor(navigator, "webdriver");
              if (desc && typeof desc.get === "function") {
                var gs = Function.prototype.toString.call(desc.get);
                if (gs && !/\[native code\]/.test(gs)) hits += 1;
              }
            } catch (e5) {}
            try {
              // CDP Runtime often leaves Error.stack getter non-native under some injectors
              var es = Object.getOwnPropertyDescriptor(Error.prototype, "stack");
              if (es && es.get && !/\[native code\]/.test(Function.prototype.toString.call(es.get))) {
                hits += 1;
              }
            } catch (e6) {}
            // iss/39 R11: Permission/Notification API toString + Runtime binding leftovers
            try {
              if (typeof Notification !== "undefined") {
                var np = Notification.permission;
                var nt = Function.prototype.toString.call(Notification);
                if (nt && !/\[native code\]/.test(nt)) hits += 1;
                if (np === "denied" && navigator.webdriver === true) hits += 1;
              }
            } catch (e7) {}
            try {
              if (window.cdc_adoQpoasnfa76pfcZLmcfl_Array || window.cdc_adoQpoasnfa76pfcZLmcfl_Promise) {
                hits += 1;
              }
            } catch (e8) {}
            return hits > 0 ? hits : 0;
          } catch (e) {
            return null;
          }
        })(),
        // iss/38 P2-2: coarse worker parallelism vs cores (ratio 0..1+)
        worker_throughput_vs_cores: (function () {
          try {
            if (typeof Worker === "undefined") return null;
            var cores = navigator.hardwareConcurrency || 1;
            var n = Math.min(4, Math.max(1, cores));
            // Synchronous micro-benchmark proxy: spawn is expensive; use parallel
            // setTimeout ticks as weak stand-in when full Worker bench too heavy for static pack.
            var started = Date.now();
            var done = 0;
            for (var i = 0; i < n; i++) {
              (function () {
                try {
                  var w = new Worker(
                    URL.createObjectURL(
                      new Blob(
                        ["onmessage=function(){var x=0;for(var i=0;i<2e5;i++)x+=i;postMessage(x);}"],
                        { type: "application/javascript" }
                      )
                    )
                  );
                  w.onmessage = function () {
                    done += 1;
                    w.terminate();
                  };
                  w.postMessage(1);
                } catch (e0) {}
              })();
            }
            // Never busy-spin (Firefox: "Script terminated by timeout").
            // Workers already async; use short elapsed from start for rate.
            var elapsed = Math.max(1, Date.now() - started);
            // If workers still in-flight, yield a microtask-free estimate (best-effort).
            var rate = done / n;
            return Math.max(0, Math.min(1.5, rate * (80 / Math.max(elapsed, 16))));
          } catch (e) {
            return null;
          }
        })(),
        // iss/21 T-ENG-1 + engine-aware probe profiles (not commercial identity)
        // InstallTrigger is deprecated — existence check only, never evaluate value.
        install_trigger_present: installTriggerPresentQuiet(),
        safari_push_notification: !!(window.safari && window.safari.pushNotification),
        engine_family: detectEngineFamily(),
        probe_profile: probeProfileForEngine(),
        engine_obs: (function () {
          // Capability-side obs; no InstallTrigger (deprecated).
          try {
            if (isGeckoEngine()) return "gecko";
            if (window.safari && window.safari.pushNotification) return "webkit";
            if (window.chrome && (window.chrome.runtime || window.chrome.app)) return "blink";
            return detectEngineFamily();
          } catch (e) {
            return "unknown";
          }
        })(),
        engine_claim: (function () {
          var ua = String(navigator.userAgent || "").toLowerCase();
          if (/firefox|fxios/.test(ua)) return "gecko";
          if (/edg\/|edgios/.test(ua)) return "blink";
          if (/opr\/|opera/.test(ua)) return "blink";
          if (/safari/.test(ua) && !/chrome|chromium|crios/.test(ua)) return "webkit";
          if (/chrome|chromium|crios/.test(ua)) return "blink";
          return detectEngineFamily();
        })(),
        plugins_length: navigator.plugins ? navigator.plugins.length : 0,
        mime_types_length: navigator.mimeTypes ? navigator.mimeTypes.length : 0,
        outer_zero: !!(window.outerWidth === 0 && window.outerHeight === 0),
        outer_width: window.outerWidth != null ? window.outerWidth : null,
        outer_height: window.outerHeight != null ? window.outerHeight : null,
        inner_width: window.innerWidth != null ? window.innerWidth : null,
        inner_height: window.innerHeight != null ? window.innerHeight : null,
        screen_width: (function () {
          var s = readScreenMetrics();
          return s.screen_width || null;
        })(),
        screen_height: (function () {
          var s = readScreenMetrics();
          return s.screen_height || null;
        })(),
        screen_fp_protection_suspect: (function () {
          return !!readScreenMetrics().screen_fp_protection_suspect;
        })(),
        // window vs screen mismatch signal
        window_screen_delta_w:
          window.outerWidth != null && readScreenMetrics().screen_width != null
            ? Math.abs(window.outerWidth - readScreenMetrics().screen_width)
            : null,
        window_screen_delta_h:
          window.outerHeight != null && readScreenMetrics().screen_height != null
            ? Math.abs(window.outerHeight - readScreenMetrics().screen_height)
            : null,
        user_agent: navigator.userAgent || "",
        orientation: (function () {
          try {
            return screen.orientation && screen.orientation.type ? screen.orientation.type : null;
          } catch (e) {
            return null;
          }
        })(),
        // iss/35–36 D-1: claim-obs only (scorers consume; never commercial digest).
        // Heuristic artifacts — relationship/tells, not brand allowlists.
        prototype_chain_tamper: (function () {
          try {
            var hits = 0;
            function notNative(fn) {
              try {
                if (typeof fn !== "function") return false;
                return !/\[native code\]/i.test(Function.prototype.toString.call(fn));
              } catch (e0) {
                return true;
              }
            }
            try {
              if (notNative(HTMLCanvasElement && HTMLCanvasElement.prototype && HTMLCanvasElement.prototype.toDataURL))
                hits += 1;
            } catch (e1) {}
            try {
              var wdDesc = Object.getOwnPropertyDescriptor(Navigator.prototype, "webdriver");
              if (wdDesc && wdDesc.get && notNative(wdDesc.get)) hits += 1;
            } catch (e2) {}
            try {
              if (notNative(Document.prototype.querySelector)) hits += 1;
            } catch (e3) {}
            try {
              // Own-property hijack of common integrity surfaces
              if (Object.prototype.hasOwnProperty.call(navigator, "webdriver")) hits += 1;
            } catch (e4) {}
            return hits > 0;
          } catch (e) {
            return false;
          }
        })(),
        fingerprint_vendor_lie: (function () {
          try {
            // Claim vs weak obs: chrome-ish UA without chrome.runtime, or outer 0 + non-headless claim.
            var ua = String(navigator.userAgent || "");
            var claimsChrome = /Chrome|Chromium|CriOS/i.test(ua) && !/HeadlessChrome/i.test(ua);
            var hasRuntime = !!(window.chrome && window.chrome.runtime);
            if (claimsChrome && !hasRuntime && !!(navigator.webdriver)) return true;
            if (window.outerWidth === 0 && window.outerHeight === 0 && claimsChrome) return true;
            return false;
          } catch (e) {
            return false;
          }
        })(),
        antidetect_vendor_hint: (function () {
          try {
            var hits = 0;
            // Global pollution / proxy leftovers common in antidetect injectors (family tells).
            // Do not getOwnPropertyNames(window) / for-in window (Firefox fullScreen/onmoz deprecations).
            var suspects = [
              "__puppeteer_evaluation_script__",
              "__playwright",
              "__fxdriver_unwrapped",
              "cdc_adoQpoasnfa76pfcZLmcfl_Array",
              "$cdc_asdjflasutopfhvcZLmcfl_",
              "_phantom",
              "callPhantom",
              "__nightmare",
              "domAutomation",
              "domAutomationController",
            ];
            var si;
            for (si = 0; si < suspects.length; si++) {
              try {
                if (Object.prototype.hasOwnProperty.call(window, suspects[si])) {
                  hits += 1;
                  break;
                }
              } catch (eS) {}
            }
            // Permissions.query / plugins emptied while claiming desktop Chrome with plugins_length 0 + webdriver
            if (!!(navigator.webdriver) && (navigator.plugins ? navigator.plugins.length : 0) === 0) hits += 1;
            // Prototype integrity already counted separately; combine lightly
            try {
              var ts = Function.prototype.toString;
              if (ts && !/\[native code\]/i.test(Function.prototype.toString.call(ts))) hits += 1;
            } catch (e2) {}
            if (hits <= 0) return false;
            // String flavor for ops (not a brand verdict table)
            if (/camoufox|gobrowser|gologin|multilogin|adspower|dolphin/i.test(String(navigator.userAgent || ""))) {
              return "ua_antidetect_token";
            }
            return true;
          } catch (e) {
            return false;
          }
        })(),
        emulator_hint: (function () {
          try {
            var ua = String(navigator.userAgent || "");
            var mobileUa = /Android|iPhone|iPad|Mobile/i.test(ua);
            var touch = navigator.maxTouchPoints != null ? navigator.maxTouchPoints : 0;
            var cores = navigator.hardwareConcurrency || 0;
            var plat = String(navigator.platform || "");
            // Desktop platform + mobile UA, or mobile UA with zero touch + many cores (classic emu tells)
            if (mobileUa && /Win|Mac|Linux x86/i.test(plat)) return "form_platform_lie";
            if (mobileUa && touch <= 0 && cores >= 8) return "android_emu";
            if (/Android/i.test(ua) && /x86|i686|i386/i.test(ua + " " + plat)) return "android_emu";
            // genymotion / ranchu class only as weak capability obs, not product name gate
            try {
              var ren = "";
              var c = document.createElement("canvas");
              var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
              if (gl) {
                var dbg = gl.getExtension("WEBGL_debug_renderer_info");
                if (dbg) {
                  ren = String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "").toLowerCase();
                }
              }
              if (/swiftshader|llvmpipe|softpipe|virtio|vmware|angle \(google/i.test(ren) && mobileUa) {
                return "android_emu";
              }
            } catch (e1) {}
            return false;
          } catch (e) {
            return false;
          }
        })(),
      };
      enqueue(ctx, "B12_anti_camouflage", f, 94, "main");
    },
  });

  register("B8_gateway_early", {
    priority: 98,
    schedule: "static",
    batch_id: "B8_gateway",
    source: "gateway",
    layer: "b8",
    run: function (ctx) {
      // Prefer gv domain; fall back to apiBase. Multi-retry for coverage (B8 target >85%).
      var bases = [];
      var gw = (ctx.gwBase || "").replace(/\/$/, "");
      var api = (ctx.apiBase || "").replace(/\/$/, "");
      if (gw) bases.push(gw);
      if (api && api !== gw) bases.push(api);
      if (!bases.length) bases.push("");
      // Rotate bases across attempts
      var body = JSON.stringify({
        session_id: ctx.session_id,
        inject_path: ctx.inject_path || global.__GR_INJECT_PRIMARY__ || "nginx",
        fields: {
          user_agent: navigator.userAgent || "",
          early_kick: true,
          kicked_ms: Date.now(),
        },
      });
      function attempt(i) {
        var base = bases[i % bases.length];
        return fetch(base + "/v1/gateway/early", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: body,
          mode: "cors",
          credentials: "omit",
          keepalive: true,
        }).then(function (r) {
          if (!r.ok) throw new Error("gw_http_" + r.status);
          return r.json().then(function (j) {
            if (j && j.ok === false) throw new Error(j.error || "gw_ok_false");
            return j;
          });
        });
      }
      function withBackoff(n, delay) {
        return attempt(n).catch(function (e) {
          if (n >= 3) throw e;
          return new Promise(function (res) {
            setTimeout(res, delay);
          }).then(function () {
            return withBackoff(n + 1, Math.min(delay * 2, 800));
          });
        });
      }
      return withBackoff(0, 80).catch(function (e) {
        /* fe_diag spam removed */
        return null;
      });
    },
  });

  // CF edge static path — ONLY when inject is actually CF. Never invent source=cloudflare from the browser.
  register("edge.cf", {
    priority: 97,
    schedule: "static",
    batch_id: "B8_gateway",
    source: "cloudflare",
    layer: "b8",
    run: function (ctx) {
      var path = ctx.inject_path || "";
      if (path !== "cf_worker" && path !== "cloudflare") {
        // No-op: do not enqueue server-side source from FE. Gateway early covers non-CF paths.
        return null;
      }
      // Real CF inject still uses gateway/early for observation; server tags edge — FE must not forge cloudflare.
      return fetch(ctx.apiBase + "/v1/gateway/early", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          session_id: ctx.session_id,
          inject_path: "cf_worker",
          fields: {
            user_agent: navigator.userAgent || "",
            early_kick: true,
            cf_edge: true,
            kicked_ms: Date.now(),
          },
        }),
        mode: "cors",
        credentials: "omit",
      }).then(function (r) {
        return r.json();
      });
    },
  });

  register("B11_interaction", {
    priority: 90,
    schedule: "static",
    batch_id: "B11_interaction",
    layer: "deep",
    run: function (ctx) {
      // Continuous RPA on MAIN only. Multi-source RPA is bound inside nest_frame / worker
      // (sandbox_tree) with their own event streams — never re-tag main events as iframe/worker.
      // Does NOT wait for dynamic route_plan.
      function pageUrl() {
        try {
          return String(location.href || "");
        } catch (e) {
          return "";
        }
      }
      var page_id = (ctx && (ctx.page_id || (ctx.fields && ctx.fields.page_id))) || null;
      var page_url = pageUrl();

      if (global.GRRpaMonitor && typeof global.GRRpaMonitor.bind === "function") {
        var st = global.GRRpaMonitor.bind({
          source: "main",
          session_id: ctx && ctx.session_id,
          page_id: page_id,
          page_url: page_url,
          inject_path: ctx && ctx.inject_path,
          queue: ctx && ctx.queue,
          win: global,
          doc: typeof document !== "undefined" ? document : null,
          boundKey: "__GR_RPA_BOUND_main__",
        });
        if (ctx) {
          ctx._behaviorBound = true;
          ctx._behaviorEvents = (st && st.events) || [];
        }
        global.__GR_RPA_BOUND__ = true;
        global.__GR_RPA_EVENTS__ = (st && st.events) || [];
        return;
      }

      // Fallback if rpa_monitor.js not loaded: main-only bind (same contract, no multi-source re-tag).
      var events = (ctx && ctx._behaviorEvents) || global.__GR_RPA_EVENTS__ || [];
      global.__GR_RPA_EVENTS__ = events;
      var RPA_IDLE_MS = 30000;
      function flushRpa(reason, force) {
        var now = Date.now();
        var hiding = reason === "pagehide" || reason === "hidden";
        if (hiding) {
          try {
            global.__GR_PAGE_HIDING__ = true;
          } catch (e) {}
        }
        var fields = {
          behavior_early_bound: true,
          behavior_events: events.slice(),
          behavior_count: events.length,
          pagehide_flush: !!hiding,
          behavior_pagehide: !!hiding,
          rpa_flush_reason: reason || "tick",
          rpa_idle_flush: reason === "idle_30s",
          page_id: page_id,
          page_url: page_url,
          collected_at: now,
          rpa_source: "main",
          sandbox_kind: "main",
        };
        global.__GR_RPA_LAST_UPLOAD_MS__ = now;
        try {
          if (ctx.queue && ctx.queue.enqueue) {
            ctx.queue.enqueue({
              session_id: ctx.session_id,
              batch_id: "B11_interaction",
              source: "main",
              inject_path: ctx.inject_path,
              priority: hiding ? 100 : 88,
              force: !!force || hiding,
              payload: { fields: fields, sandbox_kind: "main" },
            });
          } else {
            enqueue(ctx, "B11_interaction", fields, hiding ? 100 : 88, "main");
          }
        } catch (eQ) {}
      }
      if (!global.__GR_RPA_BOUND__) {
        global.__GR_RPA_BOUND__ = true;
        function push(kind, ev) {
          if (events.length >= 80) events.shift();
          events.push({
            t: Date.now(),
            kind: kind,
            type: kind,
            source: "main",
            x: (ev && ev.clientX) || 0,
            y: (ev && ev.clientY) || 0,
          });
          global.__GR_RPA_LAST_EVENT_MS__ = Date.now();
        }
        [
          "pointerdown",
          "pointermove",
          "pointerup",
          "scroll",
          "keydown",
          "keyup",
          "click",
          "touchstart",
        ].forEach(function (type) {
          document.addEventListener(
            type,
            function (ev) {
              push(type, ev);
            },
            { passive: true, capture: true }
          );
        });
        global.__GR_RPA_TICK__ = setInterval(function () {
          if (global.__GR_PAGE_HIDING__) return;
          var now = Date.now();
          var lastEv = global.__GR_RPA_LAST_EVENT_MS__ || 0;
          var lastUp = global.__GR_RPA_LAST_UPLOAD_MS__ || 0;
          if (events.length && lastEv > lastUp && now - lastUp >= 2000) {
            flushRpa("continuous", true);
          } else if (
            events.length &&
            lastUp &&
            now - lastUp >= RPA_IDLE_MS &&
            now - lastEv >= RPA_IDLE_MS
          ) {
            flushRpa("idle_30s", true);
          }
        }, 1000);
        var flushOnce = function () {
          flushRpa("pagehide", true);
        };
        global.addEventListener("pagehide", flushOnce);
        global.addEventListener("beforeunload", flushOnce);
        global.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "hidden") flushOnce();
        });
      }
      flushRpa("bind", false);
      if (ctx) {
        ctx._behaviorBound = true;
        ctx._behaviorEvents = events;
      }
    },
  });

  /* B7_sandbox moved to registry.static.hard.js */

  function midFields(tag) {
    var f = {
      mid_tag: tag,
      hardware_concurrency: navigator.hardwareConcurrency || null,
      device_memory: navigator.deviceMemory || null,
      timezone: (function () {
        try {
          return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
        } catch (e) {
          return "";
        }
      })(),
      collected_at: Date.now(),
    };
    // iss/46 H3: stamp governed GL fields so unmasked semantics stay isolated
    try {
      if (global.GRGlGovernor && typeof global.GRGlGovernor.snapshotFields === "function") {
        var gov = global.GRGlGovernor.snapshotFields();
        Object.keys(gov || {}).forEach(function (k) {
          if (gov[k] != null && gov[k] !== "") f[k] = gov[k];
        });
      } else if (global.__GR_GL_GOV_INSTALLED__) {
        f.gl_governor_active = true;
      }
    } catch (eGov) {}
    return f;
  }

  /** Mid packs return a resolved Promise so multi-tick can wait for enqueue. */
  function midEnqueue(ctx, batchId, fields, pri) {
    // Slight priority boost so mid races ahead of deep packs on short visits.
    enqueue(ctx, batchId, fields, pri != null ? pri : 70, "main");
    return Promise.resolve({ batch_id: batchId, enqueued: true });
  }

  /**
   * Best-effort OS-instance separator for B10.
   * Prefer host inject (`__GR_OS_INSTANCE__` / boot / meta) — true machine-id
   * from guest agent / MDM. Fallback: machine-stable composite (no localStorage).
   */
  function osInstanceHashQuick() {
    try {
      var inj = null;
      if (global.__GR_OS_INSTANCE__) inj = global.__GR_OS_INSTANCE__;
      else if (global.__GR_BOOT__ && global.__GR_BOOT__.os_instance) {
        inj = global.__GR_BOOT__.os_instance;
      } else if (global.__GR_BOOT__ && global.__GR_BOOT__.os_instance_hash) {
        inj = global.__GR_BOOT__.os_instance_hash;
      } else {
        try {
          var meta = document.querySelector && document.querySelector('meta[name="gr-os-instance"]');
          if (meta && meta.content) inj = meta.content;
        } catch (eM) {}
        try {
          var sc = document.currentScript;
          if (sc && sc.getAttribute) {
            var a = sc.getAttribute("data-os-instance");
            if (a) inj = a;
          }
        } catch (eS) {}
      }
      if (inj != null && String(inj).length > 0) {
        return {
          os_instance_hash: simpleHash(String(inj)).slice(0, 16),
          os_instance_source: "inject",
        };
      }
      var nav = navigator || {};
      var scr = screen || {};
      var parts = [
        nav.platform || "",
        nav.hardwareConcurrency != null ? String(nav.hardwareConcurrency) : "",
        nav.deviceMemory != null ? String(nav.deviceMemory) : "",
        scr.width != null ? String(scr.width) : "",
        scr.height != null ? String(scr.height) : "",
        scr.colorDepth != null ? String(scr.colorDepth) : "",
      ];
      try {
        parts.push(Intl.DateTimeFormat().resolvedOptions().timeZone || "");
      } catch (eTz) {
        parts.push("");
      }
      try {
        if (nav.userAgentData && nav.userAgentData.platform) {
          parts.push(String(nav.userAgentData.platform));
        }
      } catch (eUa) {}
      // sampleRate only — baseLatency is session-noisy and forked commercial mint
      // across same-host browsers when used as host separator material.
      try {
        var AC = window.AudioContext || window.webkitAudioContext;
        if (AC) {
          var ac = new AC();
          parts.push(String(ac.sampleRate || ""));
          try {
            try { if (ac.close) { var _cl2 = ac.close(); if (_cl2 && _cl2.catch) _cl2.catch(function(){}); } } catch (_eCl2) {}
          } catch (eCl) {}
        }
      } catch (eAc) {}
      return {
        os_instance_hash: simpleHash(parts.join("|")).slice(0, 16),
        os_instance_source: "proxy_composite",
      };
    } catch (e) {
      return {};
    }
  }

  /**
   * Custom hardware-noise advanced functions (not navigator API dumps).
   * Goal: same physical chip → similar curves across browser kernels/engines.
   * Different fingerprint browsers / spoofed UA / different proxy IPs should still
   * share soft similarity; equality of API fields is intentionally not required.
   * All values are measured live — no fixed sample library.
   */
  /**
   * B10 hardware noise — **full richness**, staged so high-load stages never stack.
   * Product rule: do not drop fields/ops; serialize audio seeds + yield between stages;
   * mobile only widens yield gap. HW mutex keeps B2/B7/R from running concurrently.
   */
  function hwNoiseProbes() {
    var out = {
      hw_noise_algo: "gr_hw_noise_v4_full_staged",
      hw_noise_curves: {},
      probe_mobile_like: probeIsMobileLike(),
    };

    function stageAudio() {
      try {
        var OAC = window.OfflineAudioContext || window.webkitOfflineAudioContext;
        if (!OAC) return Promise.resolve();
        // Full buffer depth (8192) + dual seed — serial renders (not concurrent OAC thrash).
        var len = 8192;
        var sr = 44100;
        function renderAudioSeed(freq, type) {
          var ctx = new OAC(1, len, sr);
          var osc = ctx.createOscillator();
          osc.type = type || "triangle";
          osc.frequency.value = freq;
          var comp = ctx.createDynamicsCompressor();
          comp.threshold.value = -50;
          comp.knee.value = 40;
          comp.ratio.value = 12;
          comp.attack.value = 0;
          comp.release.value = 0.25;
          osc.connect(comp);
          comp.connect(ctx.destination);
          osc.start(0);
          // WebKit remint: startRendering can hang forever after a prior OAC.
          return new Promise(function (resolve, reject) {
            var settled = false;
            var to = setTimeout(function () {
              if (settled) return;
              settled = true;
              try {
                if (ctx.close) ctx.close();
              } catch (eC) {}
              reject(new Error("oac_timeout"));
            }, 2500);
            Promise.resolve(ctx.startRendering()).then(
              function (buf) {
                if (settled) return;
                settled = true;
                clearTimeout(to);
                resolve(buf);
              },
              function (err) {
                if (settled) return;
                settled = true;
                clearTimeout(to);
                reject(err);
              }
            );
          });
        }
        function binsFromBuffer(buf) {
          var data = buf.getChannelData(0);
          var bins = [];
          var nBins = 32;
          var step = Math.floor(data.length / nBins);
          var i, j, sum, sum2, absMax;
          absMax = 1e-12;
          for (i = 0; i < nBins; i++) {
            sum = 0;
            sum2 = 0;
            var c = 0;
            for (j = 0; j < step; j++) {
              var v = data[i * step + j] || 0;
              sum += v;
              sum2 += v * v;
              c++;
              if (Math.abs(v) > absMax) absMax = Math.abs(v);
            }
            var mean = sum / (c || 1);
            var rms = Math.sqrt(sum2 / (c || 1));
            // Higher precision: engine stacks differ at 1e-7..1e-9 under compressor
            bins.push(Math.round((mean / absMax) * 1e9) / 1e9);
            bins.push(Math.round((rms / absMax) * 1e9) / 1e9);
          }
          // Sparse float low-byte samples (engine quantization / denorm differences)
          try {
            var dvBuf = new ArrayBuffer(4);
            var dv = new DataView(dvBuf);
            var sampleAt = [17, 97, 257, 1023, 2048, 4095, 6000, 8000];
            for (i = 0; i < sampleAt.length; i++) {
              var ix = Math.min(data.length - 1, sampleAt[i]);
              var fv = data[ix] || 0;
              dv.setFloat32(0, fv, true);
              bins.push(dv.getUint8(0) / 255);
              bins.push(dv.getUint8(1) / 255);
            }
          } catch (eBits) {}
          var sliceStart = Math.min(data.length - 64, Math.floor(data.length * 0.9));
          var sliceSum = 0;
          for (i = sliceStart; i < data.length; i++) sliceSum += Math.abs(data[i] || 0);
          return { bins: bins, energy: sliceSum, len: data.length };
        }
        // Multi-path audio: triple OfflineAudio serial + seed-delta (class-floor breaker).
        // Absolute OAC bins are OS/engine-class deterministic; cross-seed delta exposes
        // stack quantization / denorm micro-diffs that same-SKU devices still share.
        function seedDeltaBins(x, y) {
          var d = [];
          var n = Math.min((x && x.bins && x.bins.length) || 0, (y && y.bins && y.bins.length) || 0);
          var i;
          for (i = 0; i < n; i++) {
            var dv = (Number(x.bins[i]) || 0) - (Number(y.bins[i]) || 0);
            d.push(Math.round(dv * 1e9) / 1e9);
          }
          // Energy ratio + length salt (stable across reloads of same stack)
          var ex = (x && x.energy) || 0;
          var ey = (y && y.energy) || 0;
          d.push(Math.round(((ex - ey) / (Math.abs(ex) + Math.abs(ey) + 1e-12)) * 1e6) / 1e6);
          return d;
        }
        function packAudioMaterial(a, b, c) {
          var merged = [];
          var k;
          for (k = 0; k < a.bins.length; k++) merged.push(a.bins[k]);
          if (b && b.bins) {
            for (k = 0; k < b.bins.length; k += 3) merged.push(b.bins[k]);
          }
          if (c && c.bins) {
            for (k = 0; k < c.bins.length; k += 4) merged.push(c.bins[k]);
          }
          if (merged.length > 96) merged = merged.slice(0, 96);
          out.hw_noise_curves.audio = merged;
          // Preferred commercial au material: seed-delta (not absolute class bins)
          var delta = [];
          if (b) {
            var dAB = seedDeltaBins(a, b);
            for (k = 0; k < dAB.length; k++) delta.push(dAB[k]);
          }
          if (c && b) {
            var dBC = seedDeltaBins(b, c);
            for (k = 0; k < Math.min(24, dBC.length); k++) delta.push(dBC[k]);
          } else if (c) {
            var dAC = seedDeltaBins(a, c);
            for (k = 0; k < Math.min(24, dAC.length); k++) delta.push(dAC[k]);
          }
          if (delta.length > 64) delta = delta.slice(0, 64);
          if (delta.length >= 8) {
            out.hw_noise_curves.audio_delta = delta;
            out.audio_seed_delta_curve = delta;
            out.audio_noise_delta = delta;
          }
          var enSum = a.energy + ((b && b.energy) || 0) + ((c && c.energy) || 0);
          var enN = 1 + (b ? 1 : 0) + (c ? 1 : 0);
          out.audio_noise_energy = Math.round((enSum / enN) * 1e9) / 1e9;
          out.audio_noise_sr = sr;
          out.audio_noise_len = a.len;
          out.audio_noise_seeds = enN;
          out.audio_noise_algo =
            enN >= 3
              ? "gr_audio_triple_seed_v4_delta"
              : enN >= 2
                ? "gr_audio_dual_seed_v4_delta"
                : "gr_audio_single_seed_v1";
          out.audio_probe_path =
            enN >= 3 ? "oac_triple_seed_delta" : enN >= 2 ? "oac_dual_seed_delta" : "oac_single_seed";
        }
        return renderAudioSeed(10000, "triangle")
          .then(function (bufA) {
            var a = binsFromBuffer(bufA);
            return yieldProbeGap().then(function () {
              return renderAudioSeed(2000, "sawtooth").then(function (bufB) {
                var b = binsFromBuffer(bufB);
                return yieldProbeGap().then(function () {
                  return renderAudioSeed(440, "square")
                    .then(function (bufC) {
                      packAudioMaterial(a, b, binsFromBuffer(bufC));
                    })
                    .catch(function () {
                      packAudioMaterial(a, b, null);
                    });
                });
              });
            });
          })
          .catch(function () {
            return renderAudioSeed(10000, "triangle")
              .then(function (buf) {
                var r = binsFromBuffer(buf);
                out.hw_noise_curves.audio = r.bins;
                out.audio_noise_energy = Math.round(r.energy * 1e6) / 1e6;
                out.audio_noise_sr = sr;
                out.audio_noise_len = r.len;
                out.audio_noise_seeds = 1;
                out.audio_noise_algo = "gr_audio_single_seed_v1";
                out.audio_probe_path = "oac_single_seed";
              })
              .catch(function () {
                // Path 3: live AudioContext short oscillator (WebKit fallback)
                try {
                  var AC = window.AudioContext || window.webkitAudioContext;
                  if (!AC) {
                    out.audio_probe_path = "failed_all";
                    return;
                  }
                  var ac = new AC();
                  out.audio_sample_rate = ac.sampleRate;
                  out.audio_noise_sr = ac.sampleRate;
                  out.audio_noise_algo = "gr_audio_live_ac_v1";
                  out.audio_noise_seeds = 0;
                  out.audio_probe_path = "live_audiocontext_sr";
                  // Minimal bins from sampleRate alone so commercial audio material not empty
                  out.hw_noise_curves.audio = [
                    ac.sampleRate / 48000,
                    ac.baseLatency != null ? ac.baseLatency : 0,
                    ac.outputLatency != null ? ac.outputLatency : 0,
                  ];
                  try {
                    try { if (ac.close) { var _cl2 = ac.close(); if (_cl2 && _cl2.catch) _cl2.catch(function(){}); } } catch (_eCl2) {}
                  } catch (eCl) {}
                } catch (eLive) {
                  out.audio_probe_path = "failed_all";
                }
              });
          });
      } catch (eA) {
        out.audio_probe_path = "failed_throw";
        return Promise.resolve();
      }
    }

    function stageCanvas() {
      try {
        var c = document.createElement("canvas");
        c.width = 128;
        c.height = 64;
        var g =
          c.getContext("2d", { willReadFrequently: true }) || c.getContext("2d");
        // Path 2: OffscreenCanvas when main canvas blocked
        if (!g && typeof OffscreenCanvas !== "undefined") {
          try {
            var oc = new OffscreenCanvas(128, 64);
            g = oc.getContext("2d");
            c = oc;
            out.canvas_probe_path = "offscreen_2d";
          } catch (eOc) {}
        }
        if (g) {
          if (!out.canvas_probe_path) out.canvas_probe_path = "canvas_2d";
          g.textBaseline = "alphabetic";
          g.font = "18px Arial";
          var grd = g.createLinearGradient(0, 0, 128, 64);
          grd.addColorStop(0, "#f60");
          grd.addColorStop(0.5, "rgba(0,102,153,0.7)");
          grd.addColorStop(1, "#069");
          g.fillStyle = grd;
          g.fillRect(0, 0, 128, 64);
          g.fillStyle = "#069";
          g.fillText("gr∇噪声χ", 2.5, 22.5);
          g.fillStyle = "rgba(102,204,0,0.55)";
          g.fillText("chip-noise", 8.25, 44.75);
          g.strokeStyle = "rgba(255,0,128,0.4)";
          g.beginPath();
          g.arc(90.3, 30.7, 18.2, 0, Math.PI * 2);
          g.stroke();
          // Multi-pattern canvas (001: ≥3 independent draws) — bins concatenated for of-slot entropy.
          function hist16(imgData) {
            var hist = new Array(16);
            var h0, px0;
            for (h0 = 0; h0 < 16; h0++) hist[h0] = 0;
            for (px0 = 0; px0 < imgData.length; px0 += 16) {
              var r0 = imgData[px0];
              var r1 = imgData[px0 + 4] || r0;
              var d0 = (r0 - r1 + 256) % 16;
              hist[d0]++;
            }
            var total0 = 0;
            for (h0 = 0; h0 < 16; h0++) total0 += hist[h0];
            var bins0 = [];
            for (h0 = 0; h0 < 16; h0++) {
              bins0.push(total0 ? Math.round((hist[h0] / total0) * 1e5) / 1e5 : 0);
            }
            return bins0;
          }
          var img = g.getImageData(0, 0, 128, 64).data;
          var canvasBins = hist16(img);
          // Pattern B: shadow + emoji text (subpixel)
          try {
            g.clearRect(0, 0, 128, 64);
            g.shadowColor = "rgba(0,0,255,0.35)";
            g.shadowBlur = 3.5;
            g.fillStyle = "#c06";
            g.font = "16px serif";
            g.fillText("Ωμξ ßß", 4.25, 28.75);
            g.shadowBlur = 0;
            var imgB = g.getImageData(0, 0, 128, 64).data;
            var binsB = hist16(imgB);
            for (var ib = 0; ib < binsB.length; ib += 2) canvasBins.push(binsB[ib]);
          } catch (ePatB) {}
          // Pattern C: quadratic curves + composite
          try {
            g.clearRect(0, 0, 128, 64);
            g.globalCompositeOperation = "multiply";
            g.strokeStyle = "rgba(20,180,90,0.7)";
            g.beginPath();
            g.moveTo(2.2, 50.4);
            g.quadraticCurveTo(40.6, 2.1, 90.3, 48.8);
            g.quadraticCurveTo(110.5, 60.2, 124.1, 12.7);
            g.stroke();
            g.globalCompositeOperation = "source-over";
            var imgC = g.getImageData(0, 0, 128, 64).data;
            var binsC = hist16(imgC);
            for (var ic = 0; ic < binsC.length; ic += 2) canvasBins.push(binsC[ic]);
          } catch (ePatC) {}
          // Pattern D: even-odd fill + rotate + CJK metrics (subpixel / font stack)
          try {
            g.clearRect(0, 0, 128, 64);
            g.save();
            g.translate(64, 32);
            g.rotate(0.35);
            g.fillStyle = "rgba(40,120,200,0.65)";
            g.beginPath();
            g.moveTo(-40, -20);
            g.lineTo(30, -15);
            g.lineTo(20, 25);
            g.closePath();
            g.fill("evenodd");
            g.restore();
            g.fillStyle = "#203";
            g.font = "14px 'Segoe UI', 'PingFang SC', 'Noto Sans CJK', sans-serif";
            g.fillText("指纹∇µ", 6.5, 52.25);
            var imgD = g.getImageData(0, 0, 128, 64).data;
            var binsD = hist16(imgD);
            for (var id = 0; id < binsD.length; id += 2) canvasBins.push(binsD[id]);
          } catch (ePatD) {}
          if (canvasBins.length > 64) canvasBins = canvasBins.slice(0, 64);
          out.hw_noise_curves.canvas = canvasBins;
          out.canvas_noise_patterns = 4;
          out.canvas_noise_algo = "gr_canvas_multipattern_v2";
          out.canvas_noise_hash = simpleHash(
            String(img[0]) + "," + img[100] + "," + img[500] + "," + img[1000] + "," + canvasBins.join(",")
          );
        } else {
          out.canvas_probe_path = out.canvas_probe_path || "failed_no_2d";
        }
      } catch (eC) {
        out.canvas_probe_path = "failed_throw";
      }
    }

    /**
     * CPU wall-clock curve — multi-workload, multi-round median fuse.
     * Feeds commercial cp (NOT deterministic jit lowbits).
     *
     * Real stability (DrawnApart / LockedApart style) — NOT coarse ms buckets:
     *   1) Fixed multi-workload sequence (shape = machine response profile)
     *   2) Per-round ratio to that round's median (cancels global load scale)
     *   3) Element-wise median across ROUNDS independent trials (kills GC spikes)
     * Server mint: hw_probe_analysis scale-inv structure+rank on cpu_timing_rounds.
     */
    function stageCpu() {
      var TOTAL = probeIsMobileLike() ? 30 : 36;
      var INNER = probeIsMobileLike() ? 12000 : 16000;
      var chunk = probeIsMobileLike() ? 2 : 4;
      // ≥3 independent rounds — PUF reliability without throwing uniqueness away
      var ROUNDS = 3;
      // 6 partially non-collinear workloads (relative cost shape = uniqueness)
      var NKINDS = 6;
      function workload(kind, inner) {
        var acc = 0;
        var n;
        if (kind === 0) {
          // transcendental / FPU
          for (n = 0; n < inner; n++) {
            acc += Math.sin(n * 0.017) * Math.cos(n * 0.013) + Math.sqrt(n % 97 + 1);
          }
        } else if (kind === 1) {
          // integer LCG / bitops
          var x = 1;
          for (n = 0; n < inner; n++) {
            x = (x * 1664525 + 1013904223) | 0;
            acc += (x & 0xffff) ^ (n * 2654435761);
          }
        } else if (kind === 2) {
          // string / alloc pressure
          var s = "gr-cpu";
          for (n = 0; n < Math.floor(inner / 8); n++) {
            s = (s + String.fromCharCode(32 + (n % 90))).slice(-48);
            acc += s.length * (n % 17);
          }
        } else if (kind === 3) {
          // sort / compare
          var arr = [];
          var m = Math.min(64, Math.floor(inner / 200));
          for (n = 0; n < m; n++) arr.push(((n * 1103515245) >>> 0) % 997);
          arr.sort(function (a, b) {
            return a - b;
          });
          acc = arr[0] + arr[arr.length - 1];
        } else if (kind === 4) {
          // branch-heavy unpredictable
          var y = 0;
          for (n = 0; n < inner; n++) {
            if ((n * 2654435761) & 1) y += n % 13;
            else if ((n * 1597334677) & 2) y ^= n;
            else y -= (n % 7) | 0;
          }
          acc = y;
        } else {
          // cache-ish sequential + stride walk on typed array
          var ta = new Float64Array(Math.min(4096, Math.floor(inner / 4)));
          for (n = 0; n < ta.length; n++) ta[n] = n * 0.001;
          var stride = 17;
          var idx = 0;
          for (n = 0; n < inner; n++) {
            idx = (idx + stride) % ta.length;
            acc += ta[idx];
            ta[idx] = acc * 1e-9;
          }
        }
        return acc;
      }
      function runOneRound() {
        var times = [];
        var k = 0;
        function step() {
          var end = Math.min(k + chunk, TOTAL);
          for (; k < end; k++) {
            var kind = k % NKINDS;
            var t0 = performance.now();
            var acc = workload(kind, INNER);
            var t1 = performance.now();
            times.push(t1 - t0);
            if (acc === Infinity) times.push(0);
          }
          if (k < TOTAL) return yieldProbeGap().then(step);
          var sorted = times.slice().sort(function (a, b) {
            return a - b;
          });
          var med = sorted[Math.floor(sorted.length / 2)] || 1;
          // Per-round ratio curve (scale-invariant vs global load)
          var ratio = times.map(function (t) {
            return Math.round((t / med) * 1e6) / 1e6;
          });
          return Promise.resolve({ raw: times, ratio: ratio, med: med });
        }
        return step();
      }
      function runRounds(left, acc) {
        if (left <= 0) {
          try {
            // Drop first round as JIT/warmup (standard microbench practice)
            var usable = acc.length > 1 ? acc.slice(1) : acc;
            var rawRounds = usable.map(function (r) {
              return r.raw;
            });
            // Per-kind multi-round median — relative cost shape uniqueness
            var kindMed = [];
            var ri, si, kind, ki;
            var kindSamples = [];
            for (ki = 0; ki < NKINDS; ki++) kindSamples.push([]);
            for (ri = 0; ri < rawRounds.length; ri++) {
              for (si = 0; si < rawRounds[ri].length; si++) {
                kind = si % NKINDS;
                if (isFinite(rawRounds[ri][si])) kindSamples[kind].push(rawRounds[ri][si]);
              }
            }
            for (kind = 0; kind < NKINDS; kind++) {
              var col = kindSamples[kind].slice().sort(function (a, b) {
                return a - b;
              });
              if (!col.length) {
                kindMed[kind] = 1;
                continue;
              }
              var mid = Math.floor(col.length / 2);
              kindMed[kind] = col.length % 2 ? col[mid] : 0.5 * (col[mid - 1] + col[mid]);
            }
            var sumK = 0;
            for (ki = 0; ki < NKINDS; ki++) sumK += kindMed[ki];
            var meanK = sumK / NKINDS || 1;
            // Long curve: tile kind ratios (server expects long curves)
            var fused = [];
            for (si = 0; si < TOTAL; si++) {
              kind = si % NKINDS;
              fused.push(Math.round((kindMed[kind] / meanK) * 1e6) / 1e6);
            }
            // Per-round kind-ratio rounds for server multiround analysis
            var kindRatioRounds = usable.map(function (r) {
              var km = [];
              var kn = [];
              for (ki = 0; ki < NKINDS; ki++) {
                km.push(0);
                kn.push(0);
              }
              var raw = r.raw || [];
              for (si = 0; si < raw.length; si++) {
                kind = si % NKINDS;
                if (isFinite(raw[si])) {
                  km[kind] += raw[si];
                  kn[kind]++;
                }
              }
              var means = km.map(function (s, i) {
                return kn[i] ? s / kn[i] : 1;
              });
              var mkSum = 0;
              for (ki = 0; ki < NKINDS; ki++) mkSum += means[ki];
              var mk = mkSum / NKINDS || 1;
              var outR = [];
              for (si = 0; si < TOTAL; si++) {
                outR.push(Math.round((means[si % NKINDS] / mk) * 1e6) / 1e6);
              }
              return outR;
            });
            out.hw_noise_curves.cpu = fused;
            out.cpu_timing_curve = fused;
            out.hw_curve_cpu = fused;
            out.cpu_kind_medians_ms = kindMed.map(function (x) {
              return Math.round(x * 1e6) / 1e6;
            });
            out.cpu_kind_count = NKINDS;
            out.cpu_timing_rounds = kindRatioRounds;
            out.hw_curve_cpu_rounds = kindRatioRounds;
            out.cpu_timing_raw_rounds = rawRounds;
            out.cpu_loop_median_ms =
              Math.round(
                (usable.reduce(function (s, r) {
                  return s + r.med;
                }, 0) /
                  usable.length) *
                  1e6
              ) / 1e6;
            out.cpu_loop_algo = "gr_cpu_curve_v3_multiround_median";
            out.cpu_loop_rounds = usable.length;
            out.cpu_loop_warmup_dropped = acc.length - usable.length;
            out.cpu_loop_samples = TOTAL;
            // Session quality markers (conf only — never identity body)
            try {
              out.document_hidden = !!(
                typeof document !== "undefined" && document.hidden
              );
              out.visibility_state =
                typeof document !== "undefined" && document.visibilityState
                  ? String(document.visibilityState)
                  : "unknown";
            } catch (eVis) {}
            try {
              var gCpu = typeof global !== "undefined" ? global : window;
              gCpu.__GR_CPU_LOOP_ALGO__ = out.cpu_loop_algo;
              gCpu.__GR_LITE_BUILD_ALGO__ = out.cpu_loop_algo;
              if (gCpu.GRCollectors && gCpu.GRCollectors.__h) {
                gCpu.GRCollectors.__h.cpu_loop_algo_id = out.cpu_loop_algo;
              }
            } catch (eAlgo) {}
          } catch (eCpu) {}
          return Promise.resolve();
        }
        return runOneRound().then(function (r) {
          acc.push(r);
          // Yield between rounds so GC / event-loop noise is independent
          return yieldProbeGap().then(function () {
            return runRounds(left - 1, acc);
          });
        });
      }
      try {
        return runRounds(ROUNDS, []);
      } catch (e0) {
        return Promise.resolve();
      }
    }

    /**
     * Diagnostic float-bit curve from fixed Math seeds.
     *
     * IMPORTANT (prod root-cause 2026-08-06): pure Math.sin/exp/tan/log of constant
     * inputs is IEEE-754 identical on every browser → curve is a global constant
     * (v5.8.120: 1 unique raw curve across all OS). Must NEVER overwrite hw_curve_cpu
     * or be preferred for commercial cp segment. Kept only as a class diagnostic.
     *
     * Uses DataView for real mantissa low bytes (old scale-trick collapsed to ~2 bins).
     */
    /**
     * iss/58 A2: WASM relaxed-SIMD / SIMD feature matrix (CPU path, no GPU/ANGLE).
     * Honest skip when validate fails — never blocks mint.
     */
    function stageWasmSimd() {
      try {
        out.wasm_probe_algo = "gr_wasm_simd_v2";
        var features = { simd: false, relaxed_simd: false, bulk_memory: false };
        function tryValidate(u8) {
          try {
            return !!(WebAssembly && WebAssembly.validate && WebAssembly.validate(u8));
          } catch (e) {
            return false;
          }
        }
        // wasm-feature-detect style SIMD module (v128.const + i8x16.shuffle subset)
        // https://github.com/GoogleChromeLabs/wasm-feature-detect — simd probe bytes
        var simdMod = new Uint8Array([
          0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11,
        ]);
        features.simd = tryValidate(simdMod);
        // bulk-memory: memory.copy opcode (0xfc 0x0a) feature-detect module
        var bulkMod = new Uint8Array([
          0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 3, 1, 0, 1, 10, 14, 1, 12, 0, 65, 0, 65, 0, 65, 0, 252, 10, 0, 0, 11,
        ]);
        features.bulk_memory = tryValidate(bulkMod);
        // relaxed-simd: do NOT equate to simd; leave false unless a dedicated probe passes.
        // JS Float32 free-vs-expanded mul-add delta (CPU FMA / precision path proxy).
        var fmaDeltas = [];
        var seeds = [0.1, 0.3, 0.7, 1.1, 1.7, 2.3, 3.1, 4.7];
        var si;
        for (si = 0; si < seeds.length; si++) {
          var a = Math.fround(seeds[si]);
          var b = Math.fround(1.0000001 + si * 0.01);
          var c = Math.fround(0.0000003 * (si + 1));
          var free = Math.fround(Math.fround(a * b) + c);
          var exp = Math.fround(a * b);
          exp = Math.fround(exp + c);
          fmaDeltas.push(Math.fround(free - exp));
        }
        features.relaxed_simd = false;
        // Timing ladder kept for diagnostics only — MUST NOT enter commercial digest
        // (wall-clock fork same machine; iss multiround path is separate for conf).
        var times = [];
        var k;
        for (k = 0; k < 8; k++) {
          var t0 = performance.now();
          var acc = 1.0000001;
          var i;
          var ta = new Float32Array(256);
          for (i = 0; i < 256; i++) ta[i] = Math.fround(i * 0.001 + k);
          for (i = 0; i < 8000; i++) {
            acc = Math.fround(acc * 1.0000003 + ta[i & 255]);
          }
          times.push(performance.now() - t0);
          if (!isFinite(acc)) times.push(0);
        }
        out.ws_simd_timing_curve = times.map(function (t) {
          return Math.round(t * 1e4) / 1e4;
        });
        out.ws_fma_delta_curve = fmaDeltas.map(function (d) {
          return d;
        });
        out.wasm_feature_matrix = features;
        // K/diagnostic digest only — server commercial cp does NOT use this as V body
        var digSrc =
          "simd=" +
          (features.simd ? 1 : 0) +
          "|rs=" +
          (features.relaxed_simd ? 1 : 0) +
          "|bm=" +
          (features.bulk_memory ? 1 : 0) +
          "|fd=" +
          fmaDeltas
            .map(function (d) {
              return Math.round(d * 1e9) / 1e9;
            })
            .join(",");
        out.ws_relaxed_simd_digest = simpleHash(digSrc);
        out.wasm_relaxed_simd_digest = out.ws_relaxed_simd_digest;
        out.wasm_simd_sig = out.ws_relaxed_simd_digest;
        out.wasm_probe_role = "K_diagnostic_not_commercial_v";
        out.wasm_probe_path = features.simd
          ? "wasm_simd_validate_fma_delta_v4_k_only"
          : "fma_delta_scalar_v4_k_only";
      } catch (eW) {
        out.wasm_probe_path = "failed";
        out.wasm_probe_err = String(eW && eW.message ? eW.message : eW);
      }
      return Promise.resolve();
    }

    function stageJitLowbits() {
      try {
        var inputs = [
          1.23456789, 0.7654321, Math.PI / 7, Math.E / 3, 0.111111111,
          2.718281828, 0.333333333, 1.414213562, 0.577215664, 2.302585092,
          0.69314718, 1.732050807, 0.261799387, 3.14159265 / 5, 0.987654321,
          1.618033988, 0.434294481, 2.236067977, 0.301029995, 1.095445115,
          0.866025403, 1.259921049, 0.707106781, 1.847759065, 0.523598775,
          2.094395102, 0.392699081, 1.570796326, 0.174532925, 2.617993877,
          0.62831853, 1.047197551,
        ];
        var bits = [];
        var buf = new ArrayBuffer(8);
        var dv = new DataView(buf);
        var i;
        for (i = 0; i < inputs.length; i++) {
          var x = inputs[i];
          var y = Math.sin(x) * Math.exp(x * 0.1) + Math.tan(x * 0.05) + Math.log(1 + Math.abs(x));
          if (!isFinite(y)) {
            bits.push(0);
            continue;
          }
          dv.setFloat64(0, y, true);
          // low 8 bits of mantissa (bytes 0 of little-endian float64 payload)
          var low = dv.getUint8(0);
          bits.push(low / 255);
        }
        out.hw_noise_curves.jit = bits;
        out.jit_lowbits_curve = bits;
        // Mark deterministic class probe — backend must not mint cp from this alone.
        out.jit_lowbits_algo = "gr_jit_lowbits_v2_diagnostic_deterministic";
        out.jit_lowbits_n = bits.length;
        out.jit_lowbits_silicon = false;
      } catch (eJ) {
        out.jit_lowbits_err = String(eJ && eJ.message ? eJ.message : eJ);
      }
      return Promise.resolve();
    }

    /**
     * rAF interval distribution → tz slot (de-correlated from cp).
     * Multi-round median of relative jitter — real reliability, not forced ms buckets.
     */
    function stageRafJitter() {
      return new Promise(function (resolve) {
        try {
          if (typeof requestAnimationFrame !== "function") {
            out.raf_probe_failed = "no_raf";
            resolve();
            return;
          }
          var maxN = probeIsMobileLike() ? 24 : 36;
          var ROUNDS = 3;
          function oneRound() {
            return new Promise(function (resOne) {
              var samples = [];
              var last = 0;
              var n = 0;
              function tick(ts) {
                if (last > 0) samples.push(ts - last);
                last = ts;
                n++;
                if (n < maxN) {
                  requestAnimationFrame(tick);
                } else {
                  var mean = 0;
                  var i;
                  for (i = 0; i < samples.length; i++) mean += samples[i];
                  mean = samples.length ? mean / samples.length : 16.67;
                  var jitter = samples.map(function (d) {
                    return Math.round(((d - mean) / Math.max(1, mean)) * 1e4) / 1e4;
                  });
                  resOne({
                    jitter: jitter.slice(0, 32),
                    intervals: samples.slice(0, 32).map(function (d) {
                      return Math.round(d * 100) / 100;
                    }),
                    mean: mean,
                  });
                }
              }
              requestAnimationFrame(tick);
            });
          }
          function elemMed(rounds) {
            if (!rounds || !rounds.length) return [];
            var dim = rounds[0].length;
            var outM = [];
            var i, j;
            for (i = 0; i < dim; i++) {
              var col = [];
              for (j = 0; j < rounds.length; j++) {
                if (rounds[j] && isFinite(rounds[j][i])) col.push(rounds[j][i]);
              }
              col.sort(function (a, b) {
                return a - b;
              });
              var mid = Math.floor(col.length / 2);
              outM.push(col.length % 2 ? col[mid] : 0.5 * (col[mid - 1] + col[mid]));
            }
            return outM;
          }
          function runAll(left, acc) {
            if (left <= 0) {
              try {
                var jRounds = acc.map(function (r) {
                  return r.jitter;
                });
                var iRounds = acc.map(function (r) {
                  return r.intervals;
                });
                var fusedJ = elemMed(jRounds);
                var fusedI = elemMed(iRounds);
                out.hw_noise_curves.timing = fusedJ;
                out.timing_jitter_curve = fusedJ;
                out.raf_interval_curve = fusedI;
                out.timing_jitter_rounds = jRounds;
                out.raf_interval_rounds = iRounds;
                // Hz estimates are K/diagnostic only (server does not mint V from hz_class).
                // Drop frame outliers (>2.5× median interval) before per-round mean;
                // commercial stability uses multiround jitter structure, not Hz buckets.
                function cleanMean(intervals) {
                  if (!intervals || !intervals.length) return 16.67;
                  var sorted = intervals
                    .filter(function (d) {
                      return isFinite(d) && d > 0;
                    })
                    .slice()
                    .sort(function (a, b) {
                      return a - b;
                    });
                  if (!sorted.length) return 16.67;
                  var medI = sorted[Math.floor(sorted.length / 2)];
                  var cap = medI * 2.5;
                  var s = 0;
                  var n = 0;
                  var i;
                  for (i = 0; i < sorted.length; i++) {
                    if (sorted[i] <= cap) {
                      s += sorted[i];
                      n++;
                    }
                  }
                  return n ? s / n : medI;
                }
                var hzList = [];
                var maxHz = 0;
                var sumMean = 0;
                acc.forEach(function (r) {
                  var cm = cleanMean(r.intervals);
                  sumMean += cm;
                  var h = cm > 0 ? 1000 / cm : 0;
                  if (h > 0 && isFinite(h)) hzList.push(h);
                  if (h > maxHz) maxHz = h;
                });
                hzList.sort(function (a, b) {
                  return a - b;
                });
                var medHz =
                  hzList.length > 0 ? hzList[Math.floor(hzList.length / 2)] : 0;
                // Prefer median of cleaned rounds (stable); max kept as diagnostic.
                out.raf_hz_est = medHz > 0 ? Math.round(medHz * 10) / 10 : null;
                out.raf_hz_max_est = maxHz > 0 ? Math.round(maxHz * 10) / 10 : null;
                out.raf_hz_mean_est =
                  acc.length && sumMean > 0
                    ? Math.round((1000 / (sumMean / acc.length)) * 10) / 10
                    : null;
                out.raf_algo = "gr_raf_jitter_v4_multiround_structure";
                out.raf_rounds = acc.length;
                try {
                  out.document_hidden = !!(
                    typeof document !== "undefined" && document.hidden
                  );
                  out.visibility_state =
                    typeof document !== "undefined" && document.visibilityState
                      ? String(document.visibilityState)
                      : "unknown";
                } catch (eVis2) {}
              } catch (eR) {
                out.raf_probe_failed = String(eR && eR.message ? eR.message : eR);
              }
              resolve();
              return;
            }
            oneRound().then(function (r) {
              acc.push(r);
              // small gap between rounds
              setTimeout(function () {
                runAll(left - 1, acc);
              }, 16);
            });
          }
          runAll(ROUNDS, []);
        } catch (e0) {
          out.raf_probe_failed = String(e0 && e0.message ? e0.message : e0);
          resolve();
        }
      });
    }

    function stageWebgl() {
      try {
        var c2 = document.createElement("canvas");
        c2.width = 64;
        c2.height = 64;
        // Multi-path GL context: webgl2 → webgl → experimental-webgl
        var gl =
          c2.getContext("webgl2", { antialias: true, preserveDrawingBuffer: true }) ||
          c2.getContext("webgl", { antialias: true, preserveDrawingBuffer: true }) ||
          c2.getContext("experimental-webgl");
        if (!gl) {
          out.webgl_probe_path = "failed_no_context";
          return Promise.resolve();
        }
        out.webgl_context_type =
          typeof WebGL2RenderingContext !== "undefined" && gl instanceof WebGL2RenderingContext
            ? "webgl2"
            : "webgl";
        var ext = gl.getExtension("WEBGL_debug_renderer_info");
        var hp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
        var mp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.MEDIUM_FLOAT);
        out.gl_high_float = hp ? [hp.precision, hp.rangeMin, hp.rangeMax] : null;
        out.gl_med_float = mp ? [mp.precision, mp.rangeMin, mp.rangeMax] : null;
        // Async multipath residual (yields between paths — avoids UI stuck on soft refresh).
        return Promise.resolve()
          .then(function () {
            return webglResidualCurveV3e();
          })
          .then(function (v3e) {
            if (v3e && v3e.curve && v3e.curve.length >= 16) {
              out.hw_noise_curves.webgl = v3e.curve;
              out.residual_algo = v3e.residual_algo || "gr_webgl_residual_std_v3f_multipath";
              out.webgl_probe_path = "residual_multipath_v1_async";
              if (v3e.residual_probe_engine) out.residual_probe_engine = v3e.residual_probe_engine;
              if (v3e.residual_probe_profile) out.residual_probe_profile = v3e.residual_probe_profile;
              if (v3e.residual_paths) out.residual_paths = v3e.residual_paths;
              if (v3e.residual_select) out.residual_select = v3e.residual_select;
              // Multipath fused materials for commercial res/wg entropy
              if (v3e.webgl_residual_multipath && v3e.webgl_residual_multipath.length) {
                out.webgl_residual_multipath = v3e.webgl_residual_multipath;
              }
              if (v3e.residual_path_means && v3e.residual_path_means.length) {
                out.residual_path_means = v3e.residual_path_means;
              }
              if (v3e.residual_path_stds && v3e.residual_path_stds.length) {
                out.residual_path_stds = v3e.residual_path_stds;
              }
              if (v3e.residual_path_modes && v3e.residual_path_modes.length) {
                out.residual_path_modes = v3e.residual_path_modes;
              }
              if (v3e.residual_mean != null) {
                out.residual_mean = v3e.residual_mean;
                out.residual_available = true;
              }
              if (v3e.residual_std != null) out.residual_std = v3e.residual_std;
              if (out.residual_mean == null) {
                try {
                  var ri;
                  var rSum = 0;
                  var rN = 0;
                  var rSum2 = 0;
                  for (ri = 0; ri < v3e.curve.length; ri++) {
                    var rv = Number(v3e.curve[ri]);
                    if (!isNaN(rv) && isFinite(rv)) {
                      rSum += rv;
                      rSum2 += rv * rv;
                      rN++;
                    }
                  }
                  if (rN > 0) {
                    var rMean = rSum / rN;
                    out.residual_mean = Math.round(rMean * 1e12) / 1e12;
                    out.residual_available = true;
                    var rVar = rSum2 / rN - rMean * rMean;
                    if (rVar < 0) rVar = 0;
                    out.residual_std = Math.round(Math.sqrt(rVar) * 1e12) / 1e12;
                  }
                } catch (eRmCurve) {}
              }
              return;
            }
            // Fallback hist residual if multipath failed
            var hasDeriv = false;
            try {
              hasDeriv = !!gl.getExtension("OES_standard_derivatives");
            } catch (eDer) {
              hasDeriv = false;
            }
            var linkedN = linkWebglProgram(
              gl,
              "attribute vec2 a; void main(){ gl_Position=vec4(a,0.0,1.0); }",
              residualFsCandidates(0, hasDeriv, 64)
            );
            if (!linkedN) throw new Error("webgl_residual_link_failed");
            var prog = linkedN.prog;
            out.residual_algo = linkedN.algo || "gr_webgl_residual_hist_v3b";
            var buf = gl.createBuffer();
            gl.bindBuffer(gl.ARRAY_BUFFER, buf);
            gl.bufferData(
              gl.ARRAY_BUFFER,
              new Float32Array([-1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1]),
              gl.STATIC_DRAW
            );
            gl.useProgram(prog);
            var loc = gl.getAttribLocation(prog, "a");
            if (loc < 0) throw new Error("webgl_attrib_missing");
            gl.enableVertexAttribArray(loc);
            gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
            gl.viewport(0, 0, 64, 64);
            gl.clearColor(0, 0, 0, 1);
            gl.clear(gl.COLOR_BUFFER_BIT);
            gl.drawArrays(gl.TRIANGLES, 0, 6);
            var pixels = new Uint8Array(64 * 64 * 4);
            gl.readPixels(0, 0, 64, 64, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
            var hist = new Array(32);
            var hi;
            for (hi = 0; hi < 32; hi++) hist[hi] = 0;
            var pxi;
            for (pxi = 0; pxi < pixels.length; pxi += 16) {
              hist[(pixels[pxi] >> 3) & 31]++;
            }
            var tot = 0;
            for (hi = 0; hi < 32; hi++) tot += hist[hi];
            var glBins = [];
            for (hi = 0; hi < 32; hi++) {
              glBins.push(tot ? Math.round((hist[hi] / tot) * 1e5) / 1e5 : 0);
            }
            out.hw_noise_curves.webgl = glBins;
            out.webgl_probe_path = out.webgl_probe_path || "residual_hist_fallback";
          })
          .then(function () {
            // Path 3: if still no webgl curve, emit param-only fingerprint bins
            if (!out.hw_noise_curves.webgl || !out.hw_noise_curves.webgl.length) {
              try {
                var paramBins = [
                  Number(gl.getParameter(gl.MAX_TEXTURE_SIZE)) || 0,
                  Number(gl.getParameter(gl.MAX_RENDERBUFFER_SIZE)) || 0,
                  Number(gl.getParameter(gl.MAX_VERTEX_ATTRIBS)) || 0,
                  Number(gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS)) || 0,
                  Number(gl.getParameter(gl.MAX_VARYING_VECTORS)) || 0,
                  Number(gl.getParameter(gl.MAX_CUBE_MAP_TEXTURE_SIZE)) || 0,
                  (gl.getSupportedExtensions() || []).length,
                  hp ? hp.precision : 0,
                  mp ? mp.precision : 0,
                ];
                out.hw_noise_curves.webgl = paramBins;
                out.residual_algo = out.residual_algo || "gr_webgl_params_fallback_v1";
                out.webgl_probe_path = "params_fallback";
                var ps = 0;
                var pn = 0;
                var pxi2;
                for (pxi2 = 0; pxi2 < paramBins.length; pxi2++) {
                  if (isFinite(paramBins[pxi2])) {
                    ps += paramBins[pxi2];
                    pn++;
                  }
                }
                if (pn > 0 && out.residual_mean == null) {
                  out.residual_mean = Math.round((ps / pn) * 1e6) / 1e6;
                  out.residual_available = true;
                  out.residual_std =
                    Math.round((Math.abs(paramBins[0] % 97) / 1000) * 1e6) / 1e6;
                }
              } catch (ePf) {
                out.webgl_probe_path = out.webgl_probe_path || "failed_all";
              }
            }
            if (ext) {
              try {
                out.webgl_unmasked_renderer =
                  gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) || "";
              } catch (eR) {}
            }
            if (out.residual_mean == null) {
              try {
                var res2 = webglResidualMean();
                if (res2 && res2.residual_mean != null) {
                  out.residual_mean = res2.residual_mean;
                  out.residual_available = true;
                  if (!out.residual_algo) {
                    out.residual_algo = res2.residual_algo || "gr_webgl_residual_hist_v3b";
                  }
                }
                if (res2 && res2.residual_hist) out.residual_hist = res2.residual_hist;
              } catch (eRm) {}
            }
          })
          .then(function () {
            // Full unit surface N=8+3 (async chunked)
            return yieldProbeGap().then(function () {
              return webglUnitSurfaceCompactAsync();
            });
          })
          .then(function (unitS) {
            if (unitS) {
              if (unitS.unit_surface_id) out.unit_surface_id = unitS.unit_surface_id;
              if (unitS.unit_surface_algo) out.unit_surface_algo = unitS.unit_surface_algo;
              if (unitS.multi_seed_n != null) out.multi_seed_n = unitS.multi_seed_n;
              if (unitS.unit_multiround_stable != null)
                out.unit_multiround_stable = unitS.unit_multiround_stable;
              if (unitS.identical_draw_unique != null)
                out.identical_draw_unique = unitS.identical_draw_unique;
              if (unitS.unit_surface_available != null)
                out.unit_surface_available = unitS.unit_surface_available;
              if (!out.residual_hist && unitS.residual_hist) {
                out.residual_hist = unitS.residual_hist;
              }
            }
            if (!out.residual_hist) {
              try {
                var resH = webglResidualMean(0.02);
                if (resH && resH.residual_hist) out.residual_hist = resH.residual_hist;
                if (out.residual_mean == null && resH && resH.residual_mean != null) {
                  out.residual_mean = resH.residual_mean;
                  out.residual_available = true;
                }
              } catch (eRh) {}
            }
            try {
              var fusion2 = envStackFusion(out);
              Object.keys(fusion2).forEach(function (k) {
                out[k] = fusion2[k];
              });
            } catch (eF) {}
          });
      } catch (eG) {
        return Promise.resolve();
      }
    }

    function stageBound(p, ms, label) {
      return new Promise(function (resolve) {
        var settled = false;
        var to = setTimeout(function () {
          if (settled) return;
          settled = true;
          try {
            out[label + "_timeout"] = true;
          } catch (eT) {}
          resolve();
        }, ms);
        Promise.resolve(p).then(
          function () {
            if (settled) return;
            settled = true;
            clearTimeout(to);
            resolve();
          },
          function () {
            if (settled) return;
            settled = true;
            clearTimeout(to);
            resolve();
          }
        );
      });
    }

    // Stage order = priority of commercial materials first where possible:
    // audio/canvas → cpu → jit lowbits → raf jitter (tz) → webgl residual (heaviest last).
    // Per-stage bound: WebKit remint after extra-tab OAC/GL must not stall B10 forever.
    return stageBound(stageAudio(), 7000, "audio")
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        stageCanvas();
        return yieldProbeGap();
      })
      .then(function () {
        return stageBound(stageCpu(), 8000, "cpu");
      })
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        return stageWasmSimd();
      })
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        return stageJitLowbits();
      })
      .then(function () {
        return stageBound(stageRafJitter(), 3000, "raf");
      })
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        return stageBound(stageWebgl(), 10000, "webgl");
      })
      .then(function () {
        try {
          var fusion3 = envStackFusion(out);
          Object.keys(fusion3).forEach(function (k) {
            out[k] = fusion3[k];
          });
        } catch (eF2) {}
        return out;
      });
  }

  /**
   * Best-effort WebRTC host hash for B10 hard path (host separator).
   * Multi-pass engine-aware gather — **never invent IPs**.
   *
   * Default policy (v5.8.137): **host-only first, no public STUN** — avoids Chrome
   * Local Network Access permission prompts and limits rtc pressure. Optional deep
   * STUN only when `global.__GR_WEBRTC_ALLOW_STUN__ === true` (lab diagnostics).
   *
   * Fallback chain:
   *  1) host_only_empty_ice — host candidates without STUN (default)
   *  2) [opt-in STUN] stun_dual / stun_google / pool_all
   * Diagnostics: webrtc_method, webrtc_fallback_tried, webrtc_probe_failed, counts.
   * mDNS (.local) is hashed only as diagnostic (privacy-randomized; not commercial).
   */
  function webrtcHostHashQuick(timeoutMs) {
    return new Promise(function (resolve) {
      var engine = detectEngineFamily();
      var allowStun = false;
      try {
        allowStun = global.__GR_WEBRTC_ALLOW_STUN__ === true;
      } catch (eSt) {
        allowStun = false;
      }
      var done = false;
      var tried = [];
      function finish(out) {
        if (done) return;
        done = true;
        out = out || {};
        out.webrtc_probe_engine = engine;
        out.webrtc_probe_profile = probeProfileForEngine(engine);
        out.webrtc_fallback_tried = tried.slice();
        out.webrtc_stun_allowed = !!allowStun;
        resolve(out);
      }
      function isPrivateV4(ip) {
        var p = ip.split(".");
        if (p.length !== 4) return false;
        var a = parseInt(p[0], 10);
        var b = parseInt(p[1], 10);
        if (a === 10) return true;
        if (a === 192 && b === 168) return true;
        if (a === 172 && b >= 16 && b <= 31) return true;
        if (a === 127) return true;
        return false;
      }
      function isLinkLocalV6(ip) {
        return /^fe80:/i.test(String(ip || ""));
      }
      // Raised budgets for host gather SLA (prod webrtc fill was near-zero).
      // WebKitGTK often finishes host gather later; blink also needs STUN room.
      var baseBudget = timeoutMs != null ? timeoutMs : engine === "webkit" ? 6000 : 4000;
      try {
        var RTC = window.RTCPeerConnection || window.webkitRTCPeerConnection;
        if (!RTC) {
          finish({ webrtc_probe_failed: "no_rtc", webrtc_host_count: 0 });
          return;
        }
        var hosts = [];
        var mdnsHosts = [];
        var anyHost = false;
        var anySrflx = false;
        var method = "stun_dual";
        function pushHost(ip) {
          if (!ip || hosts.indexOf(ip) >= 0) return;
          hosts.push(ip);
        }
        function pushMdns(h) {
          if (!h || mdnsHosts.indexOf(h) >= 0) return;
          mdnsHosts.push(h);
        }
        function pack() {
          if (!hosts.length) {
            var fail = {
              webrtc_host_count: 0,
              ice_has_host: !!anyHost,
              ice_has_srflx: !!anySrflx,
              webrtc_method: method,
              webrtc_probe_failed: anyHost
                ? "host_typ_no_ip"
                : anySrflx
                  ? "srflx_only_no_host"
                  : "no_host_candidates",
            };
            // mDNS-only: conf diagnostic (not commercial host sep — ephemeral).
            if (mdnsHosts.length) {
              fail.webrtc_mdns_count = mdnsHosts.length;
              fail.webrtc_mdns_hash = simpleHash(mdnsHosts.slice().sort().join("|"));
              fail.webrtc_mdns_conf_only = true;
            }
            return fail;
          }
          var sorted = hosts.slice().sort();
          return {
            webrtc_host_ip_hash: simpleHash(sorted.join("|")),
            webrtc_host_ips: sorted.slice(0, 8),
            webrtc_host_count: hosts.length,
            ice_has_host: true,
            ice_has_srflx: !!anySrflx,
            webrtc_method: method,
          };
        }
        function runPc(cfg, label, budgetMs, onDone) {
          method = label;
          tried.push(label);
          hosts = [];
          mdnsHosts = [];
          anyHost = false;
          anySrflx = false;
          var pc;
          try {
            pc = new RTC(cfg || { iceServers: [] });
          } catch (eNew) {
            onDone({ webrtc_probe_failed: "pc_ctor_" + label });
            return;
          }
          var localDone = false;
          function end() {
            if (localDone) return;
            localDone = true;
            try {
              pc.close();
            } catch (eC) {}
            onDone(pack());
          }
          var timer = setTimeout(end, budgetMs || baseBudget);
          pc.onicecandidate = function (ev) {
            if (!ev || !ev.candidate || !ev.candidate.candidate) {
              if (ev && !ev.candidate) {
                clearTimeout(timer);
                end();
              }
              return;
            }
            var line = String(ev.candidate.candidate || "");
            var isHost = /\btyp host\b/.test(line);
            var isSrflx = /\btyp srflx\b/.test(line);
            if (isHost) anyHost = true;
            if (isSrflx) anySrflx = true;
            // IPv4
            var ipm = /([0-9]{1,3}(?:\.[0-9]{1,3}){3})/.exec(line);
            if (ipm) {
              var ip = ipm[1];
              if (isHost || isPrivateV4(ip)) pushHost(ip);
            }
            // IPv6 (host / link-local only — not public)
            var v6m = /([0-9a-fA-F:]+)/.exec(line.replace(/.*candidate:\S+\s+\d+\s+\S+\s+\d+\s+/, ""));
            // Prefer explicit address field when present
            try {
              var addr = ev.candidate.address || ev.candidate.ip || "";
              if (addr && isHost) {
                if (/:/.test(addr) && (isLinkLocalV6(addr) || addr.indexOf(":") >= 0)) {
                  // only link-local v6 for stability; skip ephemeral public v6
                  if (isLinkLocalV6(addr)) pushHost(addr.split("%")[0]);
                }
              }
            } catch (eAddr) {}
            // mDNS host (.local) — conf only
            var md = /([a-z0-9-]+\.local)/i.exec(line);
            if (md && isHost) pushMdns(md[1].toLowerCase());
            void v6m;
          };
          try {
            if (typeof pc.addEventListener === "function") {
              pc.addEventListener("icegatheringstatechange", function () {
                try {
                  if (pc.iceGatheringState === "complete") {
                    clearTimeout(timer);
                    end();
                  }
                } catch (eG) {}
              });
            }
          } catch (eLis) {}
          try {
            pc.createDataChannel("grb10");
            pc.createOffer()
              .then(function (o) {
                return pc.setLocalDescription(o);
              })
              .catch(function () {
                clearTimeout(timer);
                end();
              });
          } catch (e3) {
            clearTimeout(timer);
            end();
          }
        }
        function okHost(o) {
          return !!(o && o.webrtc_host_ip_hash);
        }
        // Host-only first (no Local Network / STUN permission UX).
        var hostBudget = Math.min(baseBudget, engine === "webkit" ? 2500 : 1500);
        runPc({ iceServers: [] }, "host_only_empty_ice", hostBudget, function (outHost) {
          if (okHost(outHost) || !allowStun) {
            finish(outHost || {});
            return;
          }
          // Opt-in deep STUN chain (lab only via __GR_WEBRTC_ALLOW_STUN__)
          var stunDual = {
            iceServers: [
              { urls: "stun:stun.l.google.com:19302" },
              { urls: "stun:stun.cloudflare.com:3478" },
            ],
          };
          var stunGoogle = {
            iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
          };
          var poolAll = {
            iceServers: [
              { urls: "stun:stun.l.google.com:19302" },
              { urls: "stun:stun1.l.google.com:19302" },
            ],
            iceCandidatePoolSize: 2,
            iceTransportPolicy: "all",
          };
          runPc(stunDual, "stun_dual", baseBudget, function (out1) {
            if (okHost(out1)) {
              finish(out1);
              return;
            }
            runPc(stunGoogle, "stun_google_only", Math.min(baseBudget, 2000), function (out3) {
              if (okHost(out3)) {
                finish(out3);
                return;
              }
              runPc(poolAll, "pool_all_policy", Math.min(baseBudget, 2500), function (out4) {
                if (okHost(out4)) finish(out4);
                else finish(out4 || out3 || out1 || outHost || {});
              });
            });
          });
        });
      } catch (e) {
        finish({ webrtc_probe_failed: String(e && e.message ? e.message : e) });
      }
    });
  }

  /* B10_hw_curves moved to registry.static.hard.js */

  /**
   * Inline B10x residual pack (must exist even if registry.b10x.js fails to load).
   * Uploads batch_id=B10x_* with residual_paths + b10x_pack markers.
   */
  function runInlineB10xPack(ctx, packId, profile, prio) {
    var f = midFields(packId || "b10x");
    f.engine_family = detectEngineFamily();
    f.probe_profile = probeProfileForEngine(f.engine_family);
    f.b10x_pack = packId;
    f.b10x_profile = profile || "silicon_noderiv";
    f.b10x_inline = true;
    f.b10x_phase = "start";
    // Start heartbeat on source "start" — must not occupy main dedupe key.
    try {
      var Q0 = (ctx && ctx.queue) || global.GRUploadQueue;
      var sid0 = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
      if (Q0 && Q0.enqueue && sid0) {
        Q0.enqueue({
          session_id: sid0,
          batch_id: packId,
          source: "start",
          inject_path: (ctx && ctx.inject_path) || "",
          priority: prio || 90,
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
    var runMp = function () {
      var mpP = webglResidualMultiPath({ profile: profile || "silicon_noderiv" });
      // Gecko: shorter multipath budget — long sync GL work triggers script timeout.
      var mpBudget = 20000;
      try {
        var uaMp = String((navigator && navigator.userAgent) || "");
        if (/Firefox\//.test(uaMp) || (typeof window !== "undefined" && typeof window.mozInnerScreenX === "number")) {
          mpBudget = 9000;
        }
      } catch (eMb) {}
      var timed = new Promise(function (resolve) {
        var done = false;
        var to = setTimeout(function () {
          if (!done) { done = true; resolve({ __timeout: true }); }
        }, mpBudget);
        Promise.resolve(mpP).then(
          function (v) { if (!done) { done = true; clearTimeout(to); resolve(v); } },
          function (e) { if (!done) { done = true; clearTimeout(to); resolve({ __error: String(e && e.message ? e.message : e) }); } }
        );
      });
      return timed
        .then(function (mp) {
          if (!mp || mp.__timeout || mp.__error) {
            f.b10x_ok = false;
            f.b10x_err = (mp && mp.__timeout) ? "multipath_timeout" : ((mp && mp.__error) || "multipath_empty");
            f.b10x_timeout = !!(mp && mp.__timeout);
            f.b10x_phase = "done";
            return midEnqueue(ctx, packId, f, prio || 90);
          }
          f.b10x_ok = !!(mp.curve && mp.curve.length >= 8);
          f.residual_paths = mp.residual_paths || [];
          f.residual_select = mp.residual_select || null;
          f.residual_probe_engine = mp.residual_probe_engine || f.engine_family;
          f.residual_probe_profile = mp.residual_probe_profile || f.probe_profile;
          f.multipath_profile = mp.multipath_profile || profile;
          f.webgl_probe_path = "b10x_inline_" + (profile || "silicon");
          // Lane-S must not last-write commercial residual_mean / hw_curve_webgl.
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
              try {
                global.__GR_MAIN_RESIDUAL_MEAN__ = mp.residual_mean;
              } catch (eRm) {}
            }
          }
          if (mp.residual_std != null) {
            if (laneSOnly) f.residual_std_lane_s = mp.residual_std;
            else f.residual_std = mp.residual_std;
          }
          // Commercial residual_algo follows Lane-C priority (noderiv > float > rint).
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
          // Seeded residual replay (fp_channel replay_hardness): same seed twice + ulp pair
          try {
            attachSeededReplayFields(f, ctx, profile);
          } catch (eSeed) {
            f.seed_replay_err = String((eSeed && eSeed.message) || eSeed);
          }
          // Honest ULP failure surface for lane_s observability
          if (profile === "silicon_ulp" && !f.b10x_ok) {
            var ulpPath = (f.residual_paths || []).find(function (p) {
              return p && (p.path_id === "ulp_eu_timing" || (p.shader_mode === "ulp" && p.eu_timing_ms));
            });
            if (ulpPath) {
              if (ulpPath.eu_timing_ms) f.eu_timing_ms = ulpPath.eu_timing_ms;
              if (ulpPath.err) f.b10x_err = f.b10x_err || ulpPath.err;
            }
            if (!f.b10x_err) {
              f.b10x_err = f.residual_paths_n > 0 ? "weak_curve" : "ulp_no_entropy";
            }
          }
          f.b10x_phase = "done";
          f.b10x_meta = {
            pack_id: packId,
            profile: profile,
            n_paths: f.residual_paths_n,
            chosen: f.residual_select && f.residual_select.chosen_path_id,
            mean: f.residual_mean,
            std: f.residual_std,
            inline: true,
            seed_residual_digest: f.seed_residual_digest || null,
          };
          return midEnqueue(ctx, packId, f, prio || 90);
        })
        .catch(function (e) {
          f.b10x_ok = false;
          f.b10x_err = String(e && e.message ? e.message : e);
          f.b10x_phase = "done";
          return midEnqueue(ctx, packId, f, prio || 90);
        })
        .then(function (r) {
          try {
            releaseWebglProbeContexts();
          } catch (eR) {}
          return r;
        });
    };
    try {
      var PL = global.GRPackLoader;
      if (PL && typeof PL.withHardwareLock === "function") {
        return PL.withHardwareLock("b10x_inline:" + packId, runMp);
      }
    } catch (eL) {}
    return runMp();
  }

  /** Derive shader k from challenge_seed / session; run coarse+ulp twice for replay digests. */
  /**
   * iss/67 B2: pure helper for multipath role seat selection (unit-testable).
   * Returns ordered roles that fit under pathCap (healthy desktop default 5).
   */
  function multipathRoleOrderForCap(pathCap, weak) {
    var cap = pathCap != null ? Number(pathCap) : weak ? 2 : 5;
    if (!(cap > 0)) cap = weak ? 2 : 5;
    // denorm before rint — Lane-S FTZ under cap≥4 and default cap=5
    var roleOrder = ["noderiv", "float", "fma", "denorm", "rint", "ulp", "other"];
    if (weak) {
      return roleOrder.slice(0, Math.max(2, Math.min(cap, 3)));
    }
    return roleOrder.slice(0, Math.min(cap, roleOrder.length));
  }
  function healthyDesktopPathCapDefault() {
    return 5;
  }

  function attachSeededReplayFields(f, ctx, profile) {
    var seed =
      (ctx && (ctx.challenge_seed || ctx.seed)) ||
      (global.__GR_CHALLENGE_SEED__ != null ? global.__GR_CHALLENGE_SEED__ : null) ||
      (global.__GR_BOOT__ && global.__GR_BOOT__.challenge_seed) ||
      (ctx && ctx.session_id) ||
      global.__GR_SESSION_ID__ ||
      "0";
    var seedNum =
      typeof seed === "number"
        ? seed
        : parseInt(String(seed).replace(/\D/g, "").slice(0, 8) || "0", 10);
    var seedK = ((seedNum % 9973) / 9973) * 3.1 + 0.17;
    f.challenge_seed_used = seed;
    f.seed_k = Math.round(seedK * 1e6) / 1e6;
    // release_gl each shot — seeded replay must not accumulate contexts
    var a = webglResidualCurveV3fRun({
      size: 64,
      warm_frames: 0,
      force_deriv: false,
      shader_mode: "noderiv",
      seed_k: seedK,
      ctx_pref: "webgl",
      release_gl: true,
    });
    var b = webglResidualCurveV3fRun({
      size: 64,
      warm_frames: 0,
      force_deriv: false,
      shader_mode: "noderiv",
      seed_k: seedK,
      ctx_pref: "webgl",
      release_gl: true,
    });
    var meanA = a && a.mean != null ? a.mean : null;
    var meanB = b && b.mean != null ? b.mean : null;
    if (meanA != null && meanB != null) {
      f.seed_residual_means = [meanA, meanB];
      f.seed_replay_agree_0p001 = Math.abs(meanA - meanB) < 0.001;
      f.seed_residual_digest =
        "sr_" +
        simpleHash(
          String(Math.round(meanA * 1000) / 1000) +
            "|" +
            String(Math.round(meanB * 1000) / 1000) +
            "|" +
            String(f.seed_k)
        ).slice(0, 12);
    } else {
      f.seed_replay_err = f.seed_replay_err || (a && a.err) || (b && b.err) || "seed_residual_fail";
    }
    // ULP fine: only on silicon_ulp pack (avoid 4× extra contexts on every B10x)
    if (profile === "silicon_ulp") {
      var u1 = webglResidualCurveV3fRun({
        size: 64,
        warm_frames: 0,
        force_deriv: false,
        shader_mode: "ulp",
        seed_k: seedK,
        timing_samples: 4,
        ctx_pref: "webgl",
        release_gl: true,
      });
      var u2 = webglResidualCurveV3fRun({
        size: 64,
        warm_frames: 0,
        force_deriv: false,
        shader_mode: "ulp",
        seed_k: seedK + 0.000001,
        timing_samples: 0,
        ctx_pref: "webgl",
        release_gl: true,
      });
      var um1 = u1 && u1.mean != null ? u1.mean : null;
      var um2 = u2 && u2.mean != null ? u2.mean : null;
      if (um1 != null) {
        f.seed_ulp_means = [um1, um2];
        f.seed_ulp_agree =
          um2 != null ? Math.abs(um1 - um2) < 1e-5 : null;
        f.seed_ulp_digest =
          "su_" +
          simpleHash(
            String(um1) + "|" + String(um2) + "|" + JSON.stringify((u1 && u1.eu_timing_ms) || [])
          ).slice(0, 12);
        if (u1.eu_timing_ms) f.seed_ulp_eu_timing_ms = u1.eu_timing_ms;
        if (u1.err && !f.b10x_err) f.b10x_err = u1.err;
      } else if (u1 && u1.err) {
        f.seed_ulp_err = u1.err;
      }
    }
  }

  function ensureInlineB10xRegistered() {
    var rows = [
      // Lane-C first (commercial primary), Lane-S deepen last.
      ["B10x_silicon_noderiv", "silicon_noderiv", 94],
      ["B10x_silicon_rint", "silicon_rint", 93],
      ["B10x_silicon_ulp", "silicon_ulp", 90],
      // iss/54 P1–P4 — fma/denorm/tex after Lane-C sealed
      ["B10x_silicon_deep", "silicon_deep", 86],
      ["B10x_webkit_gl_noise", "webkit_deep", 88],
      ["B10x_webkit_wave2_warm4", "webkit_wave2", 85],
      ["B10x_angle_crosscheck", "angle_cross", 80],
      // softgl/legacy/unknown often collide on ANGLE (same warm4 curve) — keep as hedge only.
      ["B10x_softgl_hedge", "softgl", 84],
      ["B10x_legacy_webgl1", "legacy_webgl1", 83],
      ["B10x_unknown_kernel", "unknown", 87],
    ];
    rows.forEach(function (row) {
      var id = row[0];
      var profile = row[1];
      var prio = row[2];
      if (collectors[id] && typeof collectors[id].run === "function") return;
      register(id, {
        priority: prio,
        schedule: "dynamic",
        batch_id: id,
        layer: "hard",
        pack_lane: "deepen",
        run: function (ctx) {
          return runInlineB10xPack(ctx, id, profile, prio);
        },
      });
    });
  }
  // Register immediately so packsFromRoutePlan can resolve B10x without domain script.
  try {
    ensureInlineB10xRegistered();
  } catch (eReg) {}

  /**
   * After B10: kick cpu∥audio secondary infra without waiting for analyze round-trip.
   * B47 (cpu) and B46 (audio) run parallel to gpu B10x; B18 (gpu) joins after silicon chain.
   * Always attempt B47 so sab_clock_skip lands even without COOP/COEP.
   */
  function scheduleEagerSecondaryInfra(ctx, opts) {
    opts = opts || {};
    var PL = global.GRPackLoader;
    var Q = global.GRUploadQueue;
    var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
    // order: parallel-friendly first; B18 only when includeGpu
    var ids = [];
    if (!opts.skipParallel) {
      ids.push("B47_sab_clock", "B46_audio_deep");
    }
    if (opts.includeGpu) {
      ids.push("B18_webgpu");
    }
    function runOne(id) {
      // Prefer live registry (mid may load after hard static)
      var pack =
        (global.GRCollectors && global.GRCollectors.get && global.GRCollectors.get(id)) ||
        collectors[id];
      if (!pack || typeof pack.run !== "function") return Promise.resolve(null);
      if (Q && Q.alreadySent && sid) {
        try {
          // iss/65: never force-recollect secondary once sealed — was wire storm.
          if (Q.alreadySent({ session_id: sid, batch_id: id, source: "main" })) {
            return Promise.resolve(null);
          }
        } catch (eS) {}
      }
      var runCtx = Object.assign({}, ctx || {}, {
        session_id: sid || (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "",
        queue: (ctx && ctx.queue) || Q,
      });
      var exec = function () {
        return Promise.resolve()
          .then(function () {
            return pack.run(runCtx);
          })
          .catch(function () {
            return null;
          });
      };
      // B47 skip path is sync/cheap — never wait on resource bus (cpu holder can stick).
      if (id === "B47_sab_clock") {
        return exec();
      }
      // Use resource-class lock when available so B46∥gpu don't serialize globally.
      if (PL && typeof PL.withResourceLock === "function") {
        var cls = id === "B46_audio_deep" ? "audio" : "gpu";
        return PL.withResourceLock(cls, "eager_sec:" + id, exec, "main");
      }
      if (PL && typeof PL.withHardwareLock === "function" && id === "B18_webgpu") {
        return PL.withHardwareLock("eager_sec:" + id, exec);
      }
      return exec();
    }
    function kickAll() {
      var jobs = ids.map(function (id) {
        return Promise.resolve()
          .then(function () {
            return runOne(id);
          })
          .catch(function () {
            return null;
          });
      });
      try {
        global.__GR_EAGER_SECONDARY__ = { at: Date.now(), packs: ids.slice(), opts: opts };
      } catch (eM) {}
      return Promise.all(jobs);
    }
    // Ensure mid.gpu is loaded (B18/B46/B47 live there) before kicking.
    // B47 may also be hard-inline (ensureInlineSabClock) so kick can proceed without mid.
    var ensureMid = global.ensureMidModules;
    if (typeof ensureMid === "function") {
      var plan = {
        packs: ids.map(function (id) {
          return { pack_id: id, batch_id: id };
        }),
      };
      return Promise.resolve()
        .then(function () {
          return ensureMid(plan);
        })
        .catch(function () {
          return null;
        })
        .then(function () {
          return kickAll();
        });
    }
    return kickAll();
  }

  /** After B10, serially kick silicon B10x packs under HW lock; then B18 gpu residual. */
  function scheduleEagerB10xChain(ctx) {
    try {
      ensureInlineB10xRegistered();
    } catch (eE) {}
    try {
      ensureInlineSabClockRegistered();
    } catch (eSab) {}
    // Fire B47/B46 immediately (∥ gpu) — do not wait for silicon chain.
    try {
      scheduleEagerSecondaryInfra(ctx, { skipParallel: false, includeGpu: false });
    } catch (eSec0) {}
    // iss/75: Lane-C noderiv/rint FIRST for commercial land rate, then ulp,
    // deep last. (Prior ulp-first chain let deep last-write wipe mint view.)
    // P0: engine-aware order (GRProbeMethodMatrix).
    var silicon = [
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_silicon_ulp",
      "B10x_silicon_deep",
    ];
    try {
      if (global.GRProbeMethodMatrix && GRProbeMethodMatrix.engineB10xOrder) {
        var ordL = GRProbeMethodMatrix.engineB10xOrder();
        if (ordL && ordL.length) silicon = ordL;
      }
    } catch (eOrdL) {}
    var PL = global.GRPackLoader;
    var Q = global.GRUploadQueue;
    var sid = (ctx && ctx.session_id) || global.__GR_SESSION_ID__ || "";
    function runOne(id) {
      // Use live registry — domain b10x may upgrade inline packs.
      var pack =
        (global.GRCollectors && global.GRCollectors.get && global.GRCollectors.get(id)) ||
        collectors[id];
      if (!pack || typeof pack.run !== "function") return Promise.resolve(null);
      // iss/65: skip all B10x once sealed (including deep) — clearSentKeys loop caused 30× storms.
      if (Q && Q.alreadySent && sid) {
        try {
          if (Q.alreadySent({ session_id: sid, batch_id: id, source: "main" })) {
            return Promise.resolve(null);
          }
        } catch (eS) {}
      }
      var runCtx = Object.assign({}, ctx || {}, {
        session_id: sid || (ctx && ctx.session_id) || "",
        queue: (ctx && ctx.queue) || Q,
      });
      var exec = function () {
        return Promise.resolve()
          .then(function () {
            return pack.run(runCtx);
          })
          .catch(function () {
            return null;
          });
      };
      // Prefer pack_loader kick path (honours allowKickDespiteHalt) when available.
      if (PL && typeof PL.kickAll === "function") {
        return PL.kickAll(
          [
            {
              id: id,
              pack_id: id,
              batch_id: id,
              priority: (pack && pack.priority) || 90,
              schedule: "dynamic",
              layer: "hard",
              run: function (c) {
                return pack.run(c || runCtx);
              },
            },
          ],
          runCtx
        );
      }
      if (PL && typeof PL.withHardwareLock === "function") {
        return PL.withHardwareLock("eager:" + id, exec);
      }
      return exec();
    }
    // Fire-and-forget serial chain so B10 pack resolves; HW lock still serializes.
    // First pack (noderiv) has no yield — commercial land must start ASAP.
    // On hidden/unload after noderiv: skip deep (pagehide allowlist already drops it).
    var chain = Promise.resolve();
    var shortVisitProbe = false;
    try {
      shortVisitProbe =
        !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_UNLOADING__) ||
        (typeof document !== "undefined" && document.hidden);
    } catch (eSv) {}
    silicon.forEach(function (id, idx) {
      chain = chain.then(function () {
        if (id === "B10x_silicon_deep") {
          try {
            var hideNow =
              shortVisitProbe ||
              !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_UNLOADING__) ||
              (typeof document !== "undefined" && document.hidden);
            // Deep is deepen-optional; save GPU/slots for noderiv/rint on short dwell.
            if (hideNow) return null;
          } catch (eDeep) {}
        }
        var step = function () {
          return runOne(id);
        };
        if (idx === 0) return step();
        return yieldProbeGap().then(step);
      });
    });
    // After silicon trio+deep: real B18 compute/f16 on gpu class; re-kick B47 if still missing.
    chain = chain.then(function () {
      try {
        if (
          (typeof document !== "undefined" && document.hidden) ||
          global.__GR_PAGE_UNLOADING__
        ) {
          return null;
        }
      } catch (eSkipSec) {}
      return yieldProbeGap().then(function () {
        return scheduleEagerSecondaryInfra(ctx, { skipParallel: true, includeGpu: true });
      });
    });
    chain = chain.then(function () {
      // Final B47 guarantee: if never sent, run skip/ok path once more.
      try {
        if (Q && Q.alreadySent && sid && Q.alreadySent({ session_id: sid, batch_id: "B47_sab_clock", source: "main" })) {
          return null;
        }
      } catch (eG) {}
      return scheduleEagerSecondaryInfra(ctx, { skipParallel: false, includeGpu: false });
    });
    chain.catch(function () {});
    try {
      global.__GR_EAGER_B10X__ = { at: Date.now(), packs: silicon.slice() };
    } catch (eM) {}
    // Mark SSOT so hard-tail does not overwrite with thinner chain.
    scheduleEagerB10xChain.__ssot = true;
    return chain;
  }
  try {
    scheduleEagerB10xChain.__ssot = true;
  } catch (eSs) {}

  /**
   * Hard-inline B47 so SAB skip always available without waiting for mid.gpu load.
   * Full mid.gpu may overwrite with same run body.
   */
  function ensureInlineSabClockRegistered() {
    if (collectors["B47_sab_clock"] && typeof collectors["B47_sab_clock"].run === "function") {
      return;
    }
    if (global.GRCollectors && global.GRCollectors.get && global.GRCollectors.get("B47_sab_clock")) {
      return;
    }
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
          b47_inline: true,
        };
        function finish(ok) {
          f.data_ok = ok !== false;
          enqueue(ctx, "B47_sab_clock", f, 62, "main");
        }
        if (!f.cross_origin_isolated) {
          f.sab_clock_skip = "need_coop_coep_cross_origin_isolated";
          f.sab_clock_ok = false;
          finish(true);
          return;
        }
        if (!f.has_shared_array_buffer || !f.has_atomics) {
          f.sab_clock_skip = "no_sab_or_atomics";
          f.sab_clock_ok = false;
          finish(true);
          return;
        }
        // Full Worker calibrate lives in mid.gpu; hard-inline only honest skip above.
        // If COI is on but mid not loaded yet, report pending and let mid upgrade.
        f.sab_clock_skip = "deferred_full_calibrate";
        f.sab_clock_ok = false;
        finish(true);
      },
    });
  }
  try {
    ensureInlineSabClockRegistered();
  } catch (eSab0) {}

  /**
   * iss/61 F1 — B85 fuzzy-ECC helper echo (static, lite).
   * Server ships parity Helper Data in route_plan.fuzzy_helper; boot persists it via
   * GRStorage; this pack echoes it next session so fuzzy_ecc can stabilize
   * same-machine curve micro-drift. localStorage read only — no permission surface.
   */
  register("B85_fuzzy_helper_echo", {
    priority: 88,
    schedule: "static",
    batch_id: "B85_fuzzy_helper_echo",
    layer: "lite5",
    run: function (ctx) {
      var f = { fuzzy_echo_algo: "gr_fuzzy_helper_echo_v1", collected_at: Date.now() };
      try {
        var h =
          global.GRStorage && typeof global.GRStorage.getFuzzyHelper === "function"
            ? global.GRStorage.getFuzzyHelper()
            : null;
        if (h) {
          if (h.wg && h.wg.length) f.fuzzy_helper_wg = h.wg;
          if (h.au && h.au.length) f.fuzzy_helper_au = h.au;
          f.fuzzy_helper_v = 1;
        }
      } catch (e) {}
      f.data_ok = true;
      enqueue(ctx, "B85_fuzzy_helper_echo", f, 88, "main");
      return f;
    },
  });

  function packFromId(id) {
    var d = collectors[id];
    if (!d) return null;
    return {
      id: id,
      pack_id: id,
      priority: d.priority,
      schedule: d.schedule || "static",
      batch_id: d.batch_id || id,
      source: d.source || "main",
      layer: d.layer,
      run: function (ctx) {
        return d.run(ctx);
      },
    };
  }

  function packsFromOrder(order) {
    return order
      .map(packFromId)
      .filter(function (p) {
        return !!p;
      });
  }

  /**
   * Wave-1 static: earliest evidence for open→batch SLA.
   * B0 first (surface), B1 automation, B8 gateway — enough for first analyze.
   */
  /**
   * Wave-1 short-visit: light identity race + single hardware (B2 only).
   * B2 is the only hardware pack here — kickAll HW mutex keeps it exclusive.
   * Enough for usable os/br/device analysis even if page dies in ~1–2s.
   */
  function defaultStaticWave1() {
    return packsFromOrder([
      "B8_gateway_early",
      "B0_bootstrap",
      "B1_conflict",
      "B12_anti_camouflage",
      "B2_hardware",
      "B85_fuzzy_helper_echo",
      "B3_system",
      "B11_interaction",
    ]);
  }

  /**
   * Wave-2: heavy residual then sandbox — ordered B10 → B7 (never concurrent HW).
   * Boot serializes these; brain parallel_groups also stage them as singletons.
   */
  function defaultStaticWave2() {
    return packsFromOrder([
      "B10_hw_curves",
      "B7_sandbox",
    ]);
  }

  /**
   * Soft-gated pack ids (catalog requires_soft_v2) — never FE eager-kick (A-PROBE-1 / norm/03).
   * Brain route_plan after soft_v2_ready may still schedule these.
   */
  var SOFT_GATED_PACK_IDS = {
    B16_fast_signals: 1,
    B13_authorized: 1,
    B15_cross_curves: 1,
    B6_risk: 1,
  };

  /** Full dynamic list brain may schedule (includes Soft-gated). */
  function defaultMidPacks() {
    return packsFromOrder([
      "B16_fast_signals",
      "B13_authorized",
      "B15_cross_curves",
      "B17_hw_physical",
      // iss/54 secondary silicon/infra early — before long mid tail so short dwell lands them
      "B47_sab_clock",
      "B18_webgpu",
      "B46_audio_deep",
      "B22_gpu_timer",
      "B20_challenge_seed",
      "B21_census_volume",
      "B9_network",
      "B4_mobile",
      "B5_census",
      "B14_css_protocol",
      "B19_eme_media",
      "B23_native_canvas_hedge",
      "B24_material_crosscheck",
      "B25_clock_raf",
      "B31_shader_numeric",
      "B33_caps_pressure",
      "B28_permissions_media",
      "B29_sensors_battery",
      "B30_gpu_bandwidth",
      "B34_cpu_cache_ladder",
      "B26_agent_parity",
      "B27_storage_privacy",
      "B35_dom_perf",
      "B36_raster_msaa",
      "B37_thermal_drift_lite",
      "B38_neg_dict",
      "B39_mem_pressure",
      "B40_websocket_fp",
      "B41_hid_gamepad",
      "B42_thermal_drift_full",
      "B43_errors_engine",
      "B44_speech_deep",
      "B45_display_hdr",
      "B6_risk",
    ]);
  }

  /**
   * FE eager allowlist only (no requires_soft_v2). Used when boot opts.eagerMid=true.
   * A-PROBE-1: Soft mid/deep must wait for brain Soft gate + route_plan.
   */
  function defaultEagerMidPacks() {
    return defaultMidPacks().filter(function (p) {
      var id = (p && (p.pack_id || p.id)) || "";
      return !SOFT_GATED_PACK_IDS[id];
    });
  }

  /** Full static list (wave1 + wave2). */
  function defaultStaticPacks() {
    return defaultStaticWave1().concat(defaultStaticWave2());
  }

  /** Default static lite kick list (compat alias). */
  function defaultLitePacks() {
    return defaultStaticPacks();
  }

  /**
   * Map route_plan pack entries to runnable packs; skip unknown / already kicked.
   * Packs with force_recollect=true always run (brain residual re-collect path).
   */
  function packsFromRoutePlan(routePlan, alreadyKicked) {
    alreadyKicked = alreadyKicked || {};
    var packs = (routePlan && routePlan.packs) || [];
    var out = [];
    packs.forEach(function (p) {
      var id = p.pack_id || p.id;
      if (!id) return;
      var force = p.force_recollect === true;
      if (!force && alreadyKicked[id]) return;
      var d = collectors[id];
      // alias fallbacks
      if (!d && id === "gateway.b8") d = collectors["B8_gateway_early"];
      if (!d && id === "lite.surface") d = collectors["B0_bootstrap"];
      if (!d && id === "lite.auto") d = collectors["B1_conflict"];
      if (!d && id === "lite.gpu") d = collectors["B2_hardware"];
      if (!d && id === "lite.display") d = collectors["B3_system"];
      if (!d && id === "mid.curves") d = collectors["B10_hw_curves"];
      if (!d && id === "mid.fast_signals") d = collectors["B16_fast_signals"];
      if (!d && id === "conflict.reconcile") d = collectors["B1_conflict"];
      if (!d) return;
      var bid = d.batch_id || id;
      if (!force && alreadyKicked[bid]) return;
      var effectivePriority =
        p.effective_priority != null && isFinite(Number(p.effective_priority))
          ? Number(p.effective_priority)
          : p.priority != null
            ? Number(p.priority)
            : Number(d.priority || 0);
      out.push({
        id: d === collectors[id] ? id : Object.keys(collectors).filter(function (k) { return collectors[k] === d; })[0] || id,
        pack_id: id,
        priority: effectivePriority,
        base_priority: p.priority != null ? p.priority : d.priority,
        effective_priority: effectivePriority,
        schedule: p.schedule || d.schedule || "dynamic",
        batch_id: bid,
        source: p.source || d.source || "main",
        layer: p.layer || d.layer,
        force_recollect: force,
        run: function (ctx) {
          return d.run(ctx);
        },
      });
    });
    return out;
  }

  // --- helpers exposed for registry.mid.js (on-demand dynamic packs) ---
  var __h = {
    midFields: midFields,
    midEnqueue: midEnqueue,
    enqueue: enqueue,
    machineStableSignals: machineStableSignals,
    deepMachineProbes: deepMachineProbes,
    simpleHash: simpleHash,
    simpleHash16: simpleHash16,
    probeWithFallbacks: probeWithFallbacks,
    orderPathsForEngine: orderPathsForEngine,
    enrichBatchFallbacksSync: enrichBatchFallbacksSync,
    hardwareInventoryMultiPath: hardwareInventoryMultiPath,
    hardwareInventoryAsyncFill: hardwareInventoryAsyncFill,
    webglResidualMean: webglResidualMean,
    webglResidualCurveV3e: webglResidualCurveV3e,
    webglResidualMultiPath: webglResidualMultiPath,
    webglResidualCurveV3fRun: webglResidualCurveV3fRun,
    attachSeededReplayFields: attachSeededReplayFields,
    releaseWebglProbeContexts: releaseWebglProbeContexts,
    acquireProbeGl: acquireProbeGl,
    webrtcHostHashQuick: webrtcHostHashQuick,
    osInstanceHashQuick: osInstanceHashQuick,
    detectEngineFamily: detectEngineFamily,
    isGeckoEngine: isGeckoEngine,
    installTriggerPresentQuiet: installTriggerPresentQuiet,
    readScreenMetrics: readScreenMetrics,
    probeProfileForEngine: probeProfileForEngine,
    deriveOsFamily: deriveOsFamily,
    hwNoiseProbes: hwNoiseProbes,
    // Content identity for pack reload (must match stageCpu out.cpu_loop_algo).
    cpu_loop_algo_id: "gr_cpu_curve_v3_multiround_median",
    stripRawSamples: stripRawSamples,
    shouldUploadSamples: shouldUploadSamples,
    surfaceMaterials: surfaceMaterials,
    envStackFusion: envStackFusion,
    rendererClassFromLabel: rendererClassFromLabel,
    softwareRendererHeuristic: softwareRendererHeuristic,
    webglUnitSurfaceCompact: webglUnitSurfaceCompact,
    probeIsMobileLike: probeIsMobileLike,
    probeIsWeakDevice: probeIsWeakDevice,
    yieldProbeGap: yieldProbeGap,
    webglCapsLite: webglCapsLite,
    canvasHashLite: canvasHashLite,
    mathDigestLite: mathDigestLite,
    fontPresenceSample: fontPresenceSample,
    fieldsBootstrap: fieldsBootstrap,
    identitySurfaceFields: identitySurfaceFields,
    storageQuotaClass: storageQuotaClass,
  };

  global.GRCollectors = {
    __h: __h,
    __ready: true,
    __midLoaded: false,
    register: register,
    get: function (id) {
      return collectors[id];
    },
    ids: function () {
      return Object.keys(collectors);
    },
    /** Production B10 hardware-noise probes (live only — no sample library). */
    hwNoiseProbes: hwNoiseProbes,
    defaultLitePacks: defaultLitePacks,
    defaultStaticPacks: defaultStaticPacks,
    defaultStaticWave1: defaultStaticWave1,
    defaultStaticWave2: defaultStaticWave2,
    defaultMidPacks: defaultMidPacks,
    defaultEagerMidPacks: defaultEagerMidPacks,
    SOFT_GATED_PACK_IDS: SOFT_GATED_PACK_IDS,
    packsFromRoutePlan: packsFromRoutePlan,
    scheduleEagerB10xChain: scheduleEagerB10xChain,
    scheduleEagerSecondaryInfra: scheduleEagerSecondaryInfra,
    /** iss/67: multipath path-cap / role seat helpers (pure, unit-testable). */
    multipathRoleOrderForCap: multipathRoleOrderForCap,
    healthyDesktopPathCapDefault: healthyDesktopPathCapDefault,
    attachSeededReplayFields: attachSeededReplayFields,
    /** Honest count of in-tree registered collectors (not v4's 161). */
    count: function () {
      return Object.keys(collectors).length;
    },
    staticCount: function () {
      return defaultStaticPacks().length;
    },
    executableIds: function () {
      return Object.keys(collectors);
    },
  };
  // Stamp lite build identity as soon as collectors load (before B10 runs).
  // Must match stageCpu out.cpu_loop_algo (v3 multiround) — seal rejects v2 by default.
  try {
    global.__GR_CPU_LOOP_ALGO__ = "gr_cpu_curve_v3_multiround_median";
    global.__GR_LITE_BUILD_ALGO__ = "gr_cpu_curve_v3_multiround_median";
    if (global.GRCollectors && global.GRCollectors.__h) {
      global.GRCollectors.__h.cpu_loop_algo_id = "gr_cpu_curve_v3_multiround_median";
    }
    var liteImpl =
      (typeof global.__GR_BUILD_IMPL__ !== "undefined" && global.__GR_BUILD_IMPL__) ||
      global.__GR_SERVER_PRODUCT_VERSION__ ||
      global.__GR_PRODUCT_VERSION__ ||
      "";
    if (liteImpl) {
      global.__GR_FE_LITE_IMPL__ = String(liteImpl);
      if (global.GRFeImpl && global.GRFeImpl.noteModule) {
        global.GRFeImpl.noteModule("lite", liteImpl);
      } else {
        global.__GR_FE_IMPL__ = global.__GR_FE_IMPL__ || {};
        global.__GR_FE_IMPL__.lite = String(liteImpl);
      }
    }
  } catch (eLiteId) {}
})(typeof window !== "undefined" ? window : globalThis);
