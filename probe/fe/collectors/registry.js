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
  /**
   * Yield event loop between heavy probe paths.
   * Prefer MessageChannel (FPJS-style) — Chrome throttles nested setTimeout heavily.
   * Optional cooldown ms still applied after microtask yield for GPU cool-down.
   */
  function yieldProbeGap(extraMs) {
    var ms = 4;
    try {
      if (probeIsWeakDevice()) ms = 40;
      else if (probeIsMobileLike()) ms = 20;
      else if (typeof document !== "undefined" && document.hidden) ms = 12;
      if (global.__GR_LAB_PRESSURE__) ms = Math.max(ms, 48);
      // GPI conservative / gecko: longer yield
      var ap = global.__GR_ADAPTIVE_POLICY__;
      if (ap && (ap.mode === "conservative" || ap.mode === "recovery" || ap.engine === "gecko")) {
        ms = Math.max(ms, 28);
      }
      if (extraMs != null && isFinite(Number(extraMs))) ms = Math.max(ms, Number(extraMs));
    } catch (eY) {}
    return new Promise(function (resolve) {
      var done = false;
      function finish() {
        if (done) return;
        done = true;
        if (ms > 0) setTimeout(resolve, ms);
        else resolve();
      }
      try {
        // FingerprintJS: MessageChannel avoids setTimeout clamping in background tabs.
        var ch = new MessageChannel();
        ch.port1.onmessage = function () {
          finish();
        };
        ch.port2.postMessage(0);
      } catch (eMc) {
        setTimeout(finish, 0);
      }
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
        // P1: nest vs main residual consistency (CreepJS-style lie signal, never sole veto)
        nest_main_residual: (function () {
          try {
            var mainM =
              global.__GR_MAIN_RESIDUAL_MEAN__ != null
                ? Number(global.__GR_MAIN_RESIDUAL_MEAN__)
                : null;
            var nestCmp = global.__GR_NEST_COMPARE__ || {};
            var nestM =
              nestCmp.nest_residual_mean != null
                ? Number(nestCmp.nest_residual_mean)
                : null;
            if (mainM == null || nestM == null || !isFinite(mainM) || !isFinite(nestM)) {
              return { available: false };
            }
            var rel = Math.abs(mainM - nestM) / Math.max(1e-6, Math.abs(mainM));
            return {
              available: true,
              main_residual_mean: mainM,
              nest_residual_mean: nestM,
              relative_delta: Math.round(rel * 1e4) / 1e4,
              consistency_ok: rel < 0.45,
              // hist-lite nest scale ~0.06 vs commercial ~0.26 is expected mismatch
              scale_note: nestCmp.algo_scale_mismatch || null,
            };
          } catch (eN) {
            return { available: false, err: "nest_cmp" };
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
          return ctx.startRendering();
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
     * CPU wall-clock curve — multi-workload, longer series.
     * Feeds commercial cp (NOT deterministic jit lowbits).
     * Workloads: float trig · integer mul · string hash · array sort microburst.
     */
    function stageCpu() {
      var times = [];
      var k = 0;
      var TOTAL = probeIsMobileLike() ? 28 : 32;
      var INNER = probeIsMobileLike() ? 12000 : 16000;
      var chunk = probeIsMobileLike() ? 2 : 4;
      function workload(kind, inner) {
        var acc = 0;
        var n;
        if (kind === 0) {
          for (n = 0; n < inner; n++) {
            acc += Math.sin(n * 0.017) * Math.cos(n * 0.013) + Math.sqrt(n % 97 + 1);
          }
        } else if (kind === 1) {
          var x = 1;
          for (n = 0; n < inner; n++) {
            x = (x * 1664525 + 1013904223) | 0;
            acc += (x & 0xffff) ^ (n * 2654435761);
          }
        } else if (kind === 2) {
          var s = "gr-cpu";
          for (n = 0; n < Math.floor(inner / 8); n++) {
            s = (s + String.fromCharCode(32 + (n % 90))).slice(-48);
            acc += s.length * (n % 17);
          }
        } else {
          var arr = [];
          var m = Math.min(64, Math.floor(inner / 200));
          for (n = 0; n < m; n++) arr.push(((n * 1103515245) >>> 0) % 997);
          arr.sort(function (a, b) {
            return a - b;
          });
          acc = arr[0] + arr[arr.length - 1];
        }
        return acc;
      }
      function step() {
        var end = Math.min(k + chunk, TOTAL);
        for (; k < end; k++) {
          var kind = k % 4;
          var t0 = performance.now();
          var acc = workload(kind, INNER);
          var t1 = performance.now();
          times.push(t1 - t0);
          if (acc === Infinity) times.push(0);
        }
        if (k < TOTAL) return yieldProbeGap().then(step);
        try {
          var sorted = times.slice().sort(function (a, b) {
            return a - b;
          });
          var med = sorted[Math.floor(sorted.length / 2)] || 1;
          // Keep higher precision ratios for encode_curve uniqueness
          out.hw_noise_curves.cpu = times.map(function (t) {
            return Math.round((t / med) * 1e6) / 1e6;
          });
          out.cpu_loop_median_ms = Math.round(med * 1e6) / 1e6;
          out.cpu_loop_algo = "gr_cpu_curve_v2_multiworkload";
          try {
            var gCpu = typeof global !== "undefined" ? global : window;
            gCpu.__GR_CPU_LOOP_ALGO__ = out.cpu_loop_algo;
            gCpu.__GR_LITE_BUILD_ALGO__ = out.cpu_loop_algo;
            if (gCpu.GRCollectors && gCpu.GRCollectors.__h) {
              gCpu.GRCollectors.__h.cpu_loop_algo_id = out.cpu_loop_algo;
            }
          } catch (eAlgo) {}
          out.cpu_loop_samples = TOTAL;
        } catch (eCpu) {}
        return Promise.resolve();
      }
      try {
        return step();
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
        // relaxed-simd probe: f32x4.relaxed_madd is opcode 0xfd 0x100 range — engines without
        // relaxed reject. Use alternate: try instantiate SIMD and compute free vs expanded FMA in JS.
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
        features.relaxed_simd = features.simd; // capability floor; true relaxed needs engine support
        // Timing ladder: scalar + typed array mul
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
        var digSrc =
          "simd=" +
          (features.simd ? 1 : 0) +
          "|rs=" +
          (features.relaxed_simd ? 1 : 0) +
          "|fd=" +
          fmaDeltas
            .map(function (d) {
              return Math.round(d * 1e9) / 1e9;
            })
            .join(",") +
          "|t=" +
          times
            .map(function (t) {
              return Math.round(t * 10) / 10;
            })
            .join(",");
        out.ws_relaxed_simd_digest = simpleHash(digSrc);
        out.wasm_relaxed_simd_digest = out.ws_relaxed_simd_digest;
        out.wasm_simd_sig = out.ws_relaxed_simd_digest;
        out.wasm_probe_path = features.simd ? "wasm_simd_validate_fma_delta_v2" : "fma_delta_scalar_v2";
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
     * rAF interval distribution → timing_jitter / raf_hz (tz slot; de-correlated from cp).
     */
    function stageRafJitter() {
      return new Promise(function (resolve) {
        try {
          if (typeof requestAnimationFrame !== "function") {
            out.raf_probe_failed = "no_raf";
            resolve();
            return;
          }
          var samples = [];
          var last = 0;
          var n = 0;
          var maxN = probeIsMobileLike() ? 24 : 36;
          function tick(ts) {
            if (last > 0) samples.push(ts - last);
            last = ts;
            n++;
            if (n < maxN) {
              requestAnimationFrame(tick);
            } else {
              try {
                var mean = 0;
                var i;
                for (i = 0; i < samples.length; i++) mean += samples[i];
                mean = samples.length ? mean / samples.length : 16.67;
                var jitter = samples.map(function (d) {
                  return Math.round(((d - mean) / Math.max(1, mean)) * 1e4) / 1e4;
                });
                out.hw_noise_curves.timing = jitter.slice(0, 32);
                out.timing_jitter_curve = jitter.slice(0, 32);
                out.raf_interval_curve = samples.slice(0, 32).map(function (d) {
                  return Math.round(d * 100) / 100;
                });
                out.raf_hz_est = mean > 0 ? Math.round((1000 / mean) * 10) / 10 : null;
                out.raf_algo = "gr_raf_jitter_v1";
              } catch (eR) {
                out.raf_probe_failed = String(eR && eR.message ? eR.message : eR);
              }
              resolve();
            }
          }
          requestAnimationFrame(tick);
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

    // Stage order = priority of commercial materials first where possible:
    // audio/canvas → cpu → jit lowbits → raf jitter (tz) → webgl residual (heaviest last).
    return stageAudio()
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        stageCanvas();
        return yieldProbeGap();
      })
      .then(function () {
        return stageCpu();
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
        return stageRafJitter();
      })
      .then(function () {
        return yieldProbeGap();
      })
      .then(function () {
        return stageWebgl();
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

  register("B10_hw_curves", {
    // Align catalog: static high-priority hard anchors for commercial device_id
    priority: 91,
    schedule: "static",
    batch_id: "B10_hw_curves",
    layer: "hard",
    run: function (ctx) {
      // Seal/prod path prefers v3 multiround; accept v2 as sticky-migration only.
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
        return (
          a === EXPECTED ||
          a === EXPECTED_LEGACY ||
          String(a).indexOf("gr_cpu_curve_v3_multiround") === 0 ||
          String(a).indexOf("v3_multiround") >= 0
        );
      }
      // P2 content gate: sticky old lite must reload before B10 runs.
      // Do NOT warn when live algo is already v3 (legacy gate compared only to v2).
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
      // Pack stamp must reflect content-proven packs — NEVER fall back to product_version alone
      // (that lied as fe_packs=124 while cpu_loop_algo stayed v1 from sticky lite helpers).
      try {
        var g = typeof global !== "undefined" ? global : window;
        var algoLive = liveAlgoId();
        var packsOnly = "";
        if (algoOkForStamp(algoLive)) {
          packsOnly = String(
            (g && g.__GR_FE_PACKS_VERSION__) ||
              (g && g.__GR_FE_HARD_IMPL__) ||
              (g && g.__GR_FE_LITE_IMPL__) ||
              (g && g.__GR_BUILD_IMPL__) ||
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
        f.fe_packs_version = packsOnly;
        f.fe_code_version = String((g && g.__GR_FE_CODE_VERSION__) || packsOnly || "");
        f.fe_asset_version = packsOnly || f.fe_code_version || "";
        f.fe_impl_version = packsOnly;
        f.cpu_loop_algo_build = String(
          (g && g.__GR_CPU_LOOP_ALGO__) ||
            (g && g.GRCollectors && g.GRCollectors.__h && g.GRCollectors.__h.cpu_loop_algo_id) ||
            ""
        );
        f.product_version_fe = String(
          (g && g.__GR_SERVER_PRODUCT_VERSION__) ||
            (g && g.__GR_PRODUCT_VERSION__) ||
            (g && g.__GR_BOOT__ && (g.__GR_BOOT__.product_version || g.__GR_BOOT__.version)) ||
            ""
        );
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
      // Sticky hard early-return kept old closed-over stageCpu even after lite reload.
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
      return liveHwNoiseProbes()().then(function (noise) {
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
        // Co-land GL caps with early silicon so Model-ID (class:t*) does not wait on
        // a separate B2 upload race / short-dwell bounce (OSS FPJS collects caps sync
        // with WebGL in one agent pass; v5 previously split B2 vs B10).
        try {
          var capsEarly = webglCapsLite();
          if (capsEarly && typeof capsEarly === "object") {
            Object.keys(capsEarly).forEach(function (ck) {
              if (f[ck] == null && capsEarly[ck] != null && capsEarly[ck] !== "") {
                f[ck] = capsEarly[ck];
              }
            });
            if (f.gl_max_texture_size == null && f.webgl_max_texture != null) {
              f.gl_max_texture_size = f.webgl_max_texture;
            }
            f.b10_caps_coland = true;
          }
        } catch (eCaps) {}
        var earlyEnq = midEnqueue(ctx, "B10_hw_curves", f, 91);
        // Kick Lane-C immediately after early B10 enqueue (was 40ms) — short visits
        // often leave before the timer; noderiv must race the dwell window.
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
        }, 0);
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
    var silicon = [
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
      "B10x_silicon_ulp",
      "B10x_silicon_deep",
    ];
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

  register("B16_fast_signals", {
    priority: 70,
    schedule: "dynamic",
    batch_id: "B16_fast_signals",
    layer: "mid4",
    run: function (ctx) {
      var f = midFields("fast_signals");
      var ms = machineStableSignals();
      f.connection = ms.net_effective_type || null;
      f.downlink = ms.net_downlink;
      f.net_rtt = ms.net_rtt;
      f.net_type = ms.net_type || "";
      f.audio_sample_rate = ms.audio_sample_rate;
      f.screen_avail_width = ms.screen_avail_width;
      f.screen_avail_height = ms.screen_avail_height;
      // Performance timeline lite (nav timing)
      try {
        var nav = performance.getEntriesByType && performance.getEntriesByType("navigation");
        if (nav && nav[0]) {
          f.perf_dom_content_loaded_ms =
            nav[0].domContentLoadedEventEnd != null
              ? Math.round(nav[0].domContentLoadedEventEnd * 100) / 100
              : null;
          f.perf_load_event_ms =
            nav[0].loadEventEnd != null ? Math.round(nav[0].loadEventEnd * 100) / 100 : null;
          f.perf_ttfb_ms =
            nav[0].responseStart != null && nav[0].requestStart != null
              ? Math.round((nav[0].responseStart - nav[0].requestStart) * 100) / 100
              : null;
        }
        f.perf_now = performance.now();
        f.time_origin = performance.timeOrigin || null;
      } catch (eP) {}
      // Media prefs lite
      try {
        f.media_prefers_color_scheme = window.matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light";
        f.media_prefers_reduced_motion = !!window.matchMedia("(prefers-reduced-motion: reduce)")
          .matches;
      } catch (eM) {}
      // Merge live deep probes (WebRTC/storage/UA-CH) — real network + machine params.
      return deepMachineProbes().then(function (deep) {
        if (deep) {
          Object.keys(deep).forEach(function (k) {
            f[k] = deep[k];
          });
          if (deep.architecture) f.ua_ch_architecture = deep.architecture;
          if (deep.platform_version) f.ua_ch_platform_version = deep.platform_version;
          if (deep.storage_quota != null) f.storage_quota = deep.storage_quota;
        }
        return midEnqueue(ctx, "B16_fast_signals", f, 70);
      });
    },
  });

  register("B13_authorized", {
    priority: 68,
    schedule: "dynamic",
    batch_id: "B13_authorized",
    layer: "mid4",
    run: function (ctx) {
      return midEnqueue(
        ctx,
        "B13_authorized",
        {
          authorized_surface: true,
          cookie_enabled: !!navigator.cookieEnabled,
          local_storage: (function () {
            try {
              localStorage.setItem("__gr", "1");
              localStorage.removeItem("__gr");
              return true;
            } catch (e) {
              return false;
            }
          })(),
        },
        68
      );
    },
  });

  register("B15_cross_curves", {
    priority: 66,
    schedule: "dynamic",
    batch_id: "B15_cross_curves",
    layer: "mid4",
    run: function (ctx) {
      // Real cross-context curve compare (top vs sandbox mirror vs worker if present).
      var f = midFields("cross_curves");
      f.cross_context_algo = "gr_cross_curves_v3";
      try {
        var mainRes = null;
        try {
          mainRes = webglResidualMean();
        } catch (e0) {}
        f.main_residual_mean = mainRes && mainRes.residual_mean != null ? mainRes.residual_mean : null;
        f.main_residual_hist = mainRes && mainRes.residual_hist ? mainRes.residual_hist : null;
        var sb = global.__GR_SANDBOX_RESULT__ || {};
        f.sandbox_sources = sb.received || [];
        f.sandbox_residual_mean =
          sb.residual_mean != null
            ? sb.residual_mean
            : sb.fields && sb.fields.residual_mean != null
              ? sb.fields.residual_mean
              : null;
        // Prefer nest_residual_mean (honest nest lite) over any legacy residual_mean.
        f.nest_residual_mean =
          sb.nest_residual_mean != null
            ? sb.nest_residual_mean
            : sb.fields && sb.fields.nest_residual_mean != null
              ? sb.fields.nest_residual_mean
              : f.sandbox_residual_mean;
        f.nest_engine_family =
          sb.nest_engine_family ||
          (sb.fields && sb.fields.nest_engine_family) ||
          null;
        if (
          f.main_residual_mean != null &&
          f.sandbox_residual_mean != null &&
          typeof f.main_residual_mean === "number" &&
          typeof f.sandbox_residual_mean === "number"
        ) {
          f.cross_residual_delta = Math.abs(f.main_residual_mean - f.sandbox_residual_mean);
          f.cross_residual_match = f.cross_residual_delta < 5e-5;
        }
        var mainMean =
          f.main_residual_mean != null
            ? f.main_residual_mean
            : global.__GR_MAIN_RESIDUAL_MEAN__;
        f.nest_residual_algo =
          f.nest_residual_algo ||
          sb.nest_residual_algo ||
          (sb.fields && sb.fields.nest_residual_algo) ||
          null;
        // Prefer same-algo nest (v3b) vs commercial residual; hist-lite is not comparable.
        if (
          mainMean != null &&
          f.nest_residual_mean != null &&
          typeof mainMean === "number" &&
          typeof f.nest_residual_mean === "number"
        ) {
          var sameAlgo = /residual_hist_v3b|webgl_residual/.test(
            String(f.nest_residual_algo || "")
          );
          var sameBand =
            (mainMean >= 0.15 && f.nest_residual_mean >= 0.15) ||
            (mainMean < 0.12 && f.nest_residual_mean < 0.12);
          f.nest_vs_main_comparable = !!(sameAlgo || sameBand);
          if (f.nest_vs_main_comparable) {
            f.nest_vs_main_agree_0p001 =
              Math.abs(mainMean - f.nest_residual_mean) < 0.001;
            f.nest_vs_main_note = sameAlgo ? "same_algo_v3b" : "magnitude_band";
          } else {
            f.nest_vs_main_agree_0p001 = null;
            f.nest_vs_main_note = "algo_scale_mismatch_hist_vs_residual";
          }
        }
        // Lightweight same-page second pass for session-internal CV (anti-noise / anti-replay lite)
        try {
          var r2 = webglResidualMean();
          if (r2 && r2.residual_mean != null && f.main_residual_mean != null) {
            f.session_residual_repeat = r2.residual_mean;
            f.session_residual_cv =
              Math.abs(r2.residual_mean - f.main_residual_mean) /
              (Math.abs(f.main_residual_mean) + 1e-12);
          }
        } catch (e1) {}
        f.worker_available = typeof Worker !== "undefined";
        f.offscreen_available = typeof OffscreenCanvas !== "undefined";
        // H14 multi-field layer divergence (main vs sandbox mirror fields if present)
        try {
          var sbFields = (sb && sb.fields) || {};
          var keys = [
            "platform",
            "user_agent",
            "hardware_concurrency",
            "device_memory",
            "timezone",
            "webdriver",
            "webgl_unmasked_renderer",
          ];
          var diffs = [];
          var compared = 0;
          keys.forEach(function (k) {
            var a = f[k];
            if (a === undefined && ctx && ctx.fields) a = ctx.fields[k];
            var b = sbFields[k];
            if (a === undefined || b === undefined || b === null) return;
            compared++;
            var sa = String(a);
            var sbv = String(b);
            if (sa !== sbv) diffs.push({ key: k, main: sa.slice(0, 80), nest: sbv.slice(0, 80) });
          });
          f.layer_divergence_compared = compared;
          f.layer_divergence_n = diffs.length;
          f.layer_divergence_score =
            compared > 0 ? Math.round((1 - diffs.length / compared) * 1000) / 1000 : null;
          f.layer_divergence_sample = diffs.slice(0, 8);
          f.layer_divergence_match = compared > 0 && diffs.length === 0;
          // Scorer multi-source hedge + greepng-style named mismatches (useful→used).
          f.multi_source_match_ratio =
            compared > 0 ? Math.round((1 - diffs.length / compared) * 1000) / 1000 : null;
          f.iframe_ua_mismatch = diffs.some(function (d) { return d.key === "user_agent"; });
          f.iframe_platform_mismatch = diffs.some(function (d) { return d.key === "platform"; });
          f.sandbox_ua_mismatch = f.iframe_ua_mismatch;
          f.sandbox_platform_mismatch = f.iframe_platform_mismatch;
          f.sandbox_webdriver_mismatch = diffs.some(function (d) { return d.key === "webdriver"; });
          f.sandbox_hw_mismatch = diffs.some(function (d) {
            return d.key === "hardware_concurrency" || d.key === "device_memory";
          });
          f.sandbox_tostring_diverged = false;
          try {
            var mainTs = Function.prototype.toString.call(Function.prototype.toString);
            var nestTs = sbFields.function_tostring_sample;
            if (nestTs != null && String(nestTs) !== String(mainTs).slice(0, 120)) {
              f.sandbox_tostring_diverged = true;
            }
          } catch (eTs) {}
        } catch (eDiv) {
          f.layer_divergence_error = String((eDiv && eDiv.message) || eDiv);
        }
      } catch (eX) {
        f.cross_error = String((eX && eX.message) || eX);
      }
      f.cross_context_algo = "gr_cross_curves_v3";
      return midEnqueue(ctx, "B15_cross_curves", f, 66);
    },
  });

  register("B6_risk", {
    priority: 40,
    schedule: "dynamic",
    batch_id: "B6_risk",
    layer: "deep",
    run: function (ctx) {
      var f = {
        webdriver: !!(navigator.webdriver),
        max_touch: navigator.maxTouchPoints || 0,
        hardware_concurrency: navigator.hardwareConcurrency || null,
        plugins_length: navigator.plugins ? navigator.plugins.length : null,
        permission_notification: null,
        battery_charging: null,
        media_devices_enumerate: null,
      };
      try {
        f.automation = {
          webdriver: !!navigator.webdriver,
          playwright: !!(window._playwright || window.__playwright || navigator.webdriver),
          selenium: !!(window.document && document.$cdc_asdjflasutopfhvcZLmcfl_),
          cdc: !!(window.cdc_adoQpoasnfa76pfcZLmcfl_Array || window.cdc_adoQpoasnfa76pfcZLmcfl_Promise),
        };
      } catch (eA) {}
      var finish = function () {
        enqueue(ctx, "B6_risk", f, 40, "main");
      };
      var pending = 0;
      var done = function () {
        pending--;
        if (pending <= 0) finish();
      };
      // Readonly only — never Notification.requestPermission
      try {
        if (typeof Notification !== "undefined" && Notification.permission != null) {
          f.permission_notification = String(Notification.permission);
        }
      } catch (eP) {}
      try {
        if (navigator.getBattery) {
          pending++;
          navigator
            .getBattery()
            .then(function (b) {
              f.battery_charging = b ? !!b.charging : null;
              f.battery_level = b && b.level != null ? b.level : null;
              done();
            })
            .catch(function () {
              done();
            });
        }
      } catch (eB) {}
      try {
        if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
          pending++;
          navigator.mediaDevices
            .enumerateDevices()
            .then(function (list) {
              f.media_devices_enumerate = list ? list.length : 0;
              done();
            })
            .catch(function () {
              done();
            });
        }
      } catch (eM) {}
      if (pending === 0) finish();
    },
  });

  // --- Residual deep packs (v57/demo coverage closeout) ---
  register("B4_mobile", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B4_mobile",
    layer: "deep",
    run: function (ctx) {
      var orient = null;
      try {
        orient = screen.orientation && screen.orientation.type ? screen.orientation.type : null;
      } catch (e) {}
      var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      var f = {
        max_touch_points: navigator.maxTouchPoints != null ? navigator.maxTouchPoints : null,
        orientation: orient,
        device_pixel_ratio: typeof devicePixelRatio !== "undefined" ? devicePixelRatio : null,
        net_effective_type: conn && conn.effectiveType ? conn.effectiveType : null,
        net_rtt: conn && conn.rtt != null ? conn.rtt : null,
        net_downlink: conn && conn.downlink != null ? conn.downlink : null,
        // iss/21 U-1 / T-UA-1: UA is claim only — do NOT write form_class here
        // (B0 owns capability form_class for device_id digest).
        mobile_ua_signals: /Mobile|Android|iPhone|iPad/i.test(navigator.userAgent || ""),
        mobile_ua_claim: /Mobile|Android|iPhone|iPad/i.test(navigator.userAgent || ""),
        touch_support: "ontouchstart" in window || (navigator.maxTouchPoints || 0) > 0,
        // typeof only — do not register deviceorientation/devicemotion listeners (FF deprecation).
        sensors_motion: typeof DeviceMotionEvent !== "undefined",
        sensors_orient: typeof DeviceOrientationEvent !== "undefined",
      };
      // UA-CH high entropy when available (v57 B4 parity + fullVersionList)
      if (navigator.userAgentData && navigator.userAgentData.getHighEntropyValues) {
        navigator.userAgentData
          .getHighEntropyValues([
            "architecture",
            "model",
            "platform",
            "platformVersion",
            "bitness",
            "mobile",
            "fullVersionList",
            "wow64",
          ])
          .then(function (he) {
            f.ua_ch_architecture = he.architecture || null;
            f.ua_ch_model = he.model || null;
            f.ua_ch_platform = he.platform || null;
            f.ua_ch_platform_version = he.platformVersion || null;
            f.ua_ch_bitness = he.bitness || null;
            f.ua_ch_mobile = he.mobile != null ? !!he.mobile : null;
            f.ua_ch_wow64 = he.wow64 != null ? !!he.wow64 : null;
            if (he.fullVersionList && he.fullVersionList.length) {
              f.ua_ch_full_version_list = (he.fullVersionList || [])
                .map(function (x) {
                  return (x.brand || "") + "/" + (x.version || "");
                })
                .join("|")
                .slice(0, 256);
            }
            enqueue(ctx, "B4_mobile", f, 55, "main");
          })
          .catch(function () {
            enqueue(ctx, "B4_mobile", f, 55, "main");
          });
        return;
      }
      enqueue(ctx, "B4_mobile", f, 55, "main");
    },
  });

  register("B5_census", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B5_census",
    layer: "deep",
    run: function (ctx) {
      var intl = {};
      try {
        var ro = Intl.DateTimeFormat().resolvedOptions();
        intl.intl_locale = ro.locale || null;
        intl.intl_calendar = ro.calendar || null;
        intl.intl_numbering = ro.numberingSystem || null;
        intl.timezone = ro.timeZone || null;
      } catch (e) {}
      var voices_n = null;
      try {
        if (window.speechSynthesis) {
          var vs = speechSynthesis.getVoices() || [];
          voices_n = vs.length;
        }
      } catch (e2) {}
      // Lite census: css.supports sample + api presence flags (analysis density without 15k leaves)
      var css_supports = {};
      try {
        if (window.CSS && CSS.supports) {
          [
            ["display", "grid"],
            ["display", "flex"],
            ["color", "color(display-p3 1 0 0)"],
            ["backdrop-filter", "blur(1px)"],
            ["container-type", "inline-size"],
          ].forEach(function (pair) {
            try {
              css_supports[pair[0] + ":" + pair[1]] = !!CSS.supports(pair[0], pair[1]);
            } catch (eS) {}
          });
        }
      } catch (eC) {}
      // Single canvas sequential checks + immediate release (avoid 2 live WebGL slots).
      var api_flags = (function () {
        var flags = {
          webgl: false,
          webgl2: false,
          webgpu: !!(navigator.gpu),
          worker: typeof Worker !== "undefined",
          shared_worker: typeof SharedWorker !== "undefined",
          service_worker: !!(navigator.serviceWorker),
          offscreen: typeof OffscreenCanvas !== "undefined",
          ua_ch: !!navigator.userAgentData,
          bluetooth: !!navigator.bluetooth,
          usb: !!navigator.usb,
          hid: !!navigator.hid,
        };
        try {
          var c = document.createElement("canvas");
          var g2 = c.getContext("webgl2");
          flags.webgl2 = !!g2;
          if (g2) {
            try {
              var L2 = g2.getExtension && g2.getExtension("WEBGL_lose_context");
              if (L2 && L2.loseContext) /*lose_suppressed*/void 0;
            } catch (eL2) {}
          } else {
            var g1 = c.getContext("webgl") || c.getContext("experimental-webgl");
            flags.webgl = !!g1;
            if (g1) {
              try {
                var L1 = g1.getExtension && g1.getExtension("WEBGL_lose_context");
                if (L1 && L1.loseContext) /*lose_suppressed*/void 0;
              } catch (eL1) {}
            }
          }
          if (flags.webgl2) flags.webgl = true;
          try {
            if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
          } catch (eG) {}
        } catch (eW) {}
        return flags;
      })();
      var base = Object.assign({}, intl, {
        speech_voices_count: voices_n,
        media_canplay: !!document.createElement("video").canPlayType,
        media_canplay_mp4: (function () {
          try {
            return document.createElement("video").canPlayType('video/mp4; codecs="avc1.42E01E"') || "";
          } catch (e) {
            return "";
          }
        })(),
        css_supports: css_supports,
        api_flags: api_flags,
        window_keys_sample: windowKeysCountQuiet(),
      });
      try {
        if (navigator.storage && navigator.storage.estimate) {
          navigator.storage.estimate().then(function (est) {
            enqueue(
              ctx,
              "B5_census",
              Object.assign({}, base, {
                storage_quota: est && est.quota != null ? est.quota : null,
                storage_usage: est && est.usage != null ? est.usage : null,
              }),
              54,
              "main"
            );
          });
          return;
        }
      } catch (e3) {}
      enqueue(ctx, "B5_census", base, 54, "main");
    },
  });

  register("B9_network", {
    priority: 52,
    schedule: "dynamic",
    batch_id: "B9_network",
    layer: "deep",
    run: function (ctx) {
      var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
      var f = {
        net_rtt: conn && conn.rtt != null ? conn.rtt : null,
        net_downlink: conn && conn.downlink != null ? conn.downlink : null,
        net_effective_type: conn && conn.effectiveType ? conn.effectiveType : null,
        net_save_data: conn && conn.saveData != null ? !!conn.saveData : null,
        collected_at: Date.now(),
      };
      // DNS/cache timing lite (v57 dns_cache_timing subset): resource timing of same-origin + well-known
      try {
        var tDns0 = performance.now();
        // Use performance entries if any navigation/resource present
        var nav = performance.getEntriesByType && performance.getEntriesByType("navigation");
        if (nav && nav[0]) {
          f.dns_lookup_ms =
            nav[0].domainLookupEnd != null && nav[0].domainLookupStart != null
              ? Math.round((nav[0].domainLookupEnd - nav[0].domainLookupStart) * 100) / 100
              : null;
          f.connect_ms =
            nav[0].connectEnd != null && nav[0].connectStart != null
              ? Math.round((nav[0].connectEnd - nav[0].connectStart) * 100) / 100
              : null;
        }
        f.dns_probe_wall_ms = Math.round((performance.now() - tDns0) * 100) / 100;
      } catch (eD) {}
      var netPending = 2; // dns dual-pass + webrtc
      function finishNet() {
        netPending--;
        if (netPending > 0) return;
        enqueue(ctx, "B9_network", f, 52, "main");
      }
      // Dual-pass same-origin fetch timing delta (cache warm vs cold-ish)
      try {
        // Prefer apiBase/gv /health — avoid www CF Under Attack 403
        var healthBase = "";
        try {
          var bootN = (typeof window !== "undefined" && window.__GR_BOOT__) || {};
          healthBase = String(
            bootN.apiBase || bootN.gwBase || window.__GR_GW_DIRECT__ || ""
          ).replace(/\/$/, "");
        } catch (eHb) {}
        if (!healthBase || healthBase.charAt(0) === "/") {
          healthBase =
            typeof location !== "undefined" && location.origin ? location.origin : "";
        }
        var probeUrl = healthBase
          ? healthBase + "/health?_grdns=" + Date.now()
          : (typeof location !== "undefined" && location.origin ? location.origin : "") +
            "/health?_grdns=" +
            Date.now();
        var tA0 = performance.now();
        var dnsDone = false;
        function dnsFinish() {
          if (dnsDone) return;
          dnsDone = true;
          clearTimeout(dnsTimer);
          finishNet();
        }
        var dnsTimer = setTimeout(dnsFinish, 1200);
        fetch(probeUrl, { method: "GET", cache: "reload", credentials: "omit", mode: "cors" })
          .then(function () {
            var pass1 = performance.now() - tA0;
            var tB0 = performance.now();
            return fetch(probeUrl, {
              method: "GET",
              cache: "force-cache",
              credentials: "omit",
              mode: "cors",
            }).then(function () {
              var pass2 = performance.now() - tB0;
              f.dns_cache_timing_delta_ms =
                Math.round((pass1 - pass2) * 100) / 100;
              f.dns_probe_wall_ms =
                Math.round((pass1 + pass2) * 100) / 100;
            });
          })
          .catch(function () {})
          .then(function () {
            dnsFinish();
          });
      } catch (eDns2) {
        finishNet();
      }
      // WebRTC host hash — engine-aware gather (shared helper; commercial hash form unified)
      try {
        webrtcHostHashQuick()
          .then(function (rtc) {
            if (rtc) {
              Object.keys(rtc).forEach(function (k) {
                if (rtc[k] != null && f[k] == null) f[k] = rtc[k];
              });
              if (rtc.webrtc_host_ip_hash) f.webrtc_host_candidate = true;
            }
            finishNet();
          })
          .catch(function () {
            finishNet();
          });
      } catch (e) {
        finishNet();
      }
    },
  });

  register("B14_css_protocol", {
    priority: 50,
    schedule: "dynamic",
    batch_id: "B14_css_protocol",
    layer: "deep",
    run: function (ctx) {
      function mq(q) {
        try {
          return !!(window.matchMedia && window.matchMedia(q).matches);
        } catch (e) {
          return null;
        }
      }
      var f = {
        css_color_gamut: mq("(color-gamut: p3)") ? "p3" : mq("(color-gamut: srgb)") ? "srgb" : null,
        css_prefers_color_scheme: mq("(prefers-color-scheme: dark)")
          ? "dark"
          : mq("(prefers-color-scheme: light)")
            ? "light"
            : null,
        css_pointer_coarse: mq("(pointer: coarse)"),
        css_pointer_fine: mq("(pointer: fine)"),
        css_hover_hover: mq("(hover: hover)"),
        css_hover_none: mq("(hover: none)"),
        css_reduced_motion: mq("(prefers-reduced-motion: reduce)"),
        css_prefers_contrast: mq("(prefers-contrast: more)")
          ? "more"
          : mq("(prefers-contrast: less)")
            ? "less"
            : null,
        css_forced_colors: mq("(forced-colors: active)"),
        css_any_pointer_coarse: mq("(any-pointer: coarse)"),
        css_display_mode_standalone: mq("(display-mode: standalone)"),
        // multi_protocol lite: same-origin beacon capability flags (not full S0)
        protocol_https: typeof location !== "undefined" && location.protocol === "https:",
        protocol_beacon: typeof navigator.sendBeacon === "function",
        protocol_fetch: typeof fetch === "function",
      };
      enqueue(ctx, "B14_css_protocol", f, 50, "main");
    },
  });

  /** iss2 H03/H05/H07 (+H01/H02 lite) + multipath inventory for dh-grade device / os / br. */
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

  /** iss2 H06 — WebGPU adapter/limits + compute silicon residual (secure context).
   *  v3: true GPU compute readback curve → hw_curve_webgpu (Lane-S second silicon source).
   *  Commercial digests still from WebGL Lane-C; WebGPU is conf/secondary only.
   */
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

  /** iss2 H10 — EME / MediaCapabilities / codec matrix (VM high-cost surface). */
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

  /**
   * D37 / H16 PoHW — multi-seed residual + multi-size + audio/CPU challenge surfaces.
   * Server seed preferred; ephemeral seed when missing so path still yields materials.
   */
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

  /**
   * Deep census volume (v57 max-probe field breadth class).
   * Fonts + CSS.supports + matchMedia + API flags + CSS props + object property digests.
   * Emits compact digests + leaf_estimate (not raw 15k upload).
   */
  register("B21_census_volume", {
    priority: 53,
    schedule: "dynamic",
    batch_id: "B21_census_volume",
    layer: "deep",
    run: function (ctx) {
      var lists = global.GRProbeLists || {};
      var fonts = lists.FONTS || [];
      var fontTokens = lists.FONT_TOKENS || [];
      var mqs = lists.MEDIA_QUERIES || [];
      var cssSup = lists.CSS_SUPPORTS || [];
      var cssProps = lists.CSS_PROPS || [];
      var apis = lists.KNOWN_APIS || [];
      var f = {
        census_algo: "gr_census_volume_v2",
        collected_at: Date.now(),
        catalog_n_fonts: fonts.length,
        catalog_n_font_tokens: fontTokens.length,
        catalog_n_mq: mqs.length,
        catalog_n_css: cssSup.length,
        catalog_n_css_props: cssProps.length,
        catalog_n_apis: apis.length,
        leaf_budget_catalog: lists.leaf_budget_estimate || 0,
      };
      function censusObjectLite(obj, prefix, maxNames) {
        var out = { prefix: prefix, n_own: 0, n_proto: 0, type_hist: {}, sample: [] };
        if (!obj) return out;
        try {
          var names = Object.getOwnPropertyNames(obj);
          out.n_own = names.length;
          var types = {};
          for (var i = 0; i < names.length && i < (maxNames || 400); i++) {
            var n = names[i];
            var typ = "unknown";
            try { typ = typeof obj[n]; } catch (e) { typ = "throw"; }
            types[typ] = (types[typ] || 0) + 1;
            if (out.sample.length < 24) out.sample.push(n + ":" + typ);
          }
          out.type_hist = types;
          try {
            var proto = Object.getPrototypeOf(obj);
            if (proto) out.n_proto = Object.getOwnPropertyNames(proto).length;
          } catch (eP) {}
          out.leaf_est = out.n_own * 4 + out.n_proto;
          out.struct_hash = simpleHash(out.sample.join("|") + "|" + out.n_own).slice(0, 12);
        } catch (eC) {
          out.error = String((eC && eC.message) || eC);
        }
        return out;
      }
      try {
        var presentFonts = [];
        var base = "monospace";
        var canvas = document.createElement("canvas");
        var c2 = canvas.getContext("2d");
        if (c2 && fonts.length) {
          function fw(font) {
            c2.font = "72px " + font + "," + base;
            return c2.measureText("mmmmmmmmmmlli").width;
          }
          var baseW = fw(base);
          fonts.forEach(function (name) {
            try {
              if (Math.abs(fw("'" + name + "'") - baseW) > 0.5) presentFonts.push(name);
            } catch (e) {}
          });
        }
        f.font_present_count = presentFonts.length;
        f.font_present_sample = presentFonts.slice(0, 48);
        f.font_bitmap_hash = simpleHash(presentFonts.join("|")).slice(0, 16);
        f.font_count = presentFonts.length;
        if (fontTokens.length && c2) {
          var tokenHits = 0;
          var baseTok = fw("monospace");
          fontTokens.slice(0, 200).forEach(function (tok) {
            try {
              if (Math.abs(fw("'" + tok + "'") - baseTok) > 0.5) tokenHits++;
            } catch (e) {}
          });
          f.font_token_hit_count = tokenHits;
          f.font_token_checked = Math.min(200, fontTokens.length);
        }
      } catch (eF) {
        f.font_error = String((eF && eF.message) || eF);
      }
      try {
        var mqHits = {};
        var mqTrue = 0;
        mqs.forEach(function (q) {
          try {
            var m = window.matchMedia(q).matches;
            mqHits[q] = m;
            if (m) mqTrue++;
          } catch (e) { mqHits[q] = null; }
        });
        f.media_query_true_count = mqTrue;
        f.media_query_total = mqs.length;
        f.media_query_hash = simpleHash(
          mqs.map(function (q) { return mqHits[q] ? "1" : "0"; }).join("")
        ).slice(0, 16);
        f.media_query_true_sample = mqs.filter(function (q) { return mqHits[q]; }).slice(0, 32);
      } catch (eM) {}
      try {
        var cssBits = [];
        var cssOk = 0;
        cssSup.forEach(function (pair) {
          try {
            var ok = !!(window.CSS && CSS.supports && CSS.supports(pair[0], pair[1]));
            cssBits.push(ok ? "1" : "0");
            if (ok) cssOk++;
          } catch (e) { cssBits.push("0"); }
        });
        f.css_supports_ok_count = cssOk;
        f.css_supports_total = cssSup.length;
        f.css_supports_hash = simpleHash(cssBits.join("")).slice(0, 16);
      } catch (eC) {}
      try {
        if (cssProps.length && document.documentElement) {
          var cs = getComputedStyle(document.documentElement);
          var propBits = [];
          var propSample = {};
          cssProps.forEach(function (prop) {
            var v = "";
            try { v = cs.getPropertyValue(prop) || ""; } catch (e) { v = ""; }
            propBits.push(v ? "1" : "0");
            if (v && Object.keys(propSample).length < 20) propSample[prop] = String(v).slice(0, 48);
          });
          f.css_props_resolved_count = propBits.filter(function (b) { return b === "1"; }).length;
          f.css_props_total = cssProps.length;
          f.css_props_hash = simpleHash(propBits.join("") + "|" + JSON.stringify(propSample)).slice(0, 16);
          f.css_props_sample = propSample;
        }
      } catch (eP) {}
      try {
        var apiBits = [];
        var apiOk = 0;
        apis.forEach(function (name) {
          var ok = false;
          try {
            ok = typeof window[name] !== "undefined" || typeof navigator[name] !== "undefined";
          } catch (e) { ok = false; }
          apiBits.push(ok ? "1" : "0");
          if (ok) apiOk++;
        });
        f.api_flags_ok_count = apiOk;
        f.api_flags_total = apis.length;
        f.api_flags_hash = simpleHash(apiBits.join("")).slice(0, 16);
      } catch (eA) {}
      try {
        var censuses = {
          nav: censusObjectLite(navigator, "nav", 600),
          screen: censusObjectLite(screen, "screen", 200),
          win: censusObjectLite(window, "win", 800),
          doc: censusObjectLite(document, "doc", 500),
          perf: censusObjectLite(typeof performance !== "undefined" ? performance : null, "perf", 200),
        };
        f.object_census = {};
        var leafObj = 0;
        Object.keys(censuses).forEach(function (k) {
          var c = censuses[k];
          f.object_census[k] = {
            n_own: c.n_own,
            n_proto: c.n_proto,
            leaf_est: c.leaf_est,
            struct_hash: c.struct_hash,
            type_hist: c.type_hist,
          };
          leafObj += c.leaf_est || 0;
        });
        f.object_census_leaf_est = leafObj;
      } catch (eO) {}
      f.census_leaf_estimate =
        (f.font_present_count || 0) +
        (f.font_token_checked || 0) +
        (f.media_query_total || 0) +
        (f.css_supports_total || 0) +
        (f.css_props_total || 0) * 2 +
        (f.api_flags_total || 0) +
        (f.object_census_leaf_est || 0);
      f.data_ok =
        (f.font_present_count || 0) > 0 ||
        (f.api_flags_ok_count || 0) > 10 ||
        (f.census_leaf_estimate || 0) > 500;
      enqueue(ctx, "B21_census_volume", f, 53, "main");
    },
  });

  /**
   * H01 GPU timer: wall staircase always; GPU-ns via EXT_disjoint_timer_query.
   * Real discrete GPUs (e.g. GTX 1050 Ti) need multi-frame async rAF poll —
   * busy-spin alone often returns query_result_unavailable.
   * Honest skip only when extension missing or query never available after drain.
   */
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


  /**
   * B23 — Native integrity + canvas geometry + math/wasm hedge (D26/D02/D25).
   * Independent materials for br/device so webdriver alone cannot decide authenticity.
   */
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

  /**
   * B24 — Multi-material cross-check hedge (D31-class).
   * Compares residual/canvas/audio/challenge digests for internal consistency.
   * Emits material_vote_digest + material_cross_conflict for analysis vote.
   */
  register("B24_material_crosscheck", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B24_material_crosscheck",
    layer: "deep",
    run: function (ctx) {
      var f = {
        crosscheck_algo: "gr_material_crosscheck_v1",
        collected_at: Date.now(),
        hedge_direction: "device_id+br+os",
      };
      var materials = {};
      // Live residual mean (webgl)
      try {
        var r = webglResidualMean();
        if (r && r.residual_mean != null) {
          materials.webgl_residual = r.residual_mean;
          f.live_residual_mean = r.residual_mean;
        }
      } catch (e) {}
      // Canvas hedge digest
      try {
        var c = document.createElement("canvas");
        c.width = 64;
        c.height = 64;
        var g = c.getContext("2d");
        if (g) {
          g.fillStyle = "#123456";
          g.fillRect(0, 0, 64, 64);
          g.fillStyle = "#abcdef";
          g.fillRect(8, 8, 40, 40);
          var img = g.getImageData(0, 0, 64, 64).data;
          var s = 0;
          for (var i = 0; i < img.length; i += 8) s += img[i];
          materials.canvas_mean = Math.round((s / (img.length / 8) / 255) * 1e8) / 1e8;
          f.cross_canvas_mean = materials.canvas_mean;
        }
      } catch (e2) {}
      // Audio buffer mean
      try {
        var AC = window.AudioContext || window.webkitAudioContext;
        if (AC) {
          var ac = new AC();
          var buf = ac.createBuffer(1, 1024, ac.sampleRate || 44100);
          var data = buf.getChannelData(0);
          for (var j = 0; j < data.length; j++) data[j] = Math.sin(j * 0.05);
          var sa = 0;
          for (var k = 0; k < data.length; k++) sa += data[k];
          materials.audio_mean = Math.round((sa / data.length) * 1e8) / 1e8;
          f.cross_audio_mean = materials.audio_mean;
          try { if (ac.close) { var _cl = ac.close(); if (_cl && _cl.catch) _cl.catch(function(){}); } } catch (_eCl) {}
        }
      } catch (e3) {}
      // Prior fields from ctx.fields if brain re-runs after other packs
      var prior = (ctx && ctx.fields) || (global.__GR_FIELDS__) || {};
      if (prior.challenge_residual_mean != null) {
        materials.challenge_residual = prior.challenge_residual_mean;
      }
      if (prior.residual_mean != null && materials.webgl_residual == null) {
        materials.webgl_residual = prior.residual_mean;
      }
      var keys = Object.keys(materials);
      f.material_keys = keys;
      f.material_count = keys.length;
      f.material_vote_digest = simpleHash(
        keys
          .sort()
          .map(function (k) {
            return k + "=" + materials[k];
          })
          .join("|")
      ).slice(0, 16);
      // Cross-conflict: if we have residual + challenge residual both and they differ wildly
      // after normalizing — or canvas totally flat zero while residual present (spoof glitch)
      var conflict = false;
      var reasons = [];
      if (materials.webgl_residual != null && materials.challenge_residual != null) {
        var dlt = Math.abs(materials.webgl_residual - materials.challenge_residual);
        f.residual_challenge_delta = dlt;
        // same seed family should be different params; huge equality can be fake-constant spoof
        if (dlt < 1e-15) {
          reasons.push("residual_identical_to_challenge");
          // not always conflict — note only
        }
      }
      if (materials.canvas_mean === 0 && materials.webgl_residual != null) {
        conflict = true;
        reasons.push("canvas_flat_with_webgl");
      }
      // Consistency score: more independent materials → higher
      f.material_consistency = keys.length >= 2 ? 0.7 + 0.1 * Math.min(keys.length, 3) : 0.3;
      f.material_cross_conflict = conflict;
      f.material_cross_reasons = reasons;
      f.materials = materials;
      f.data_ok = keys.length >= 2;
      enqueue(ctx, "B24_material_crosscheck", f, 54, "main");
    },
  });


  /**
   * H08 — clock / rAF physical materials (virt timing traces).
   */
  register("B25_clock_raf", {
    priority: 51,
    schedule: "dynamic",
    batch_id: "B25_clock_raf",
    layer: "hard",
    run: function (ctx) {
      var f = {
        clock_algo: "gr_h08_clock_raf_v1",
        pohw_direction: "H08",
        collected_at: Date.now(),
      };
      try {
        f.perf_now = performance.now();
        f.time_origin = performance.timeOrigin || null;
        f.date_now_skew_ms =
          Date.now() - (performance.timeOrigin || Date.now()) - performance.now();
      } catch (e0) {}
      // performance.now resolution estimate (N deltas)
      try {
        var samples = [];
        var last = performance.now();
        for (var i = 0; i < 64; i++) {
          var cur = performance.now();
          if (cur !== last) {
            samples.push(cur - last);
            last = cur;
          }
        }
        samples.sort(function (a, b) { return a - b; });
        f.perf_now_resolution_ms =
          samples.length ? samples[Math.floor(samples.length / 2)] : null;
        f.perf_now_delta_n = samples.length;
      } catch (e1) {
        f.clock_error = String((e1 && e1.message) || e1);
      }
      // rAF jitter CV (short burst)
      try {
        if (typeof requestAnimationFrame === "function") {
          var times = [];
          var n = 0;
          var tPrev = null;
          function tick(ts) {
            if (tPrev != null) times.push(ts - tPrev);
            tPrev = ts;
            n++;
            if (n < 20) requestAnimationFrame(tick);
            else {
              if (times.length >= 4) {
                var mean =
                  times.reduce(function (s, x) { return s + x; }, 0) / times.length;
                var vsum = 0;
                times.forEach(function (x) {
                  var d = x - mean;
                  vsum += d * d;
                });
                var sd = Math.sqrt(vsum / times.length);
                f.raf_jitter_cv = mean > 0 ? sd / mean : null;
                f.raf_mean_ms = Math.round(mean * 1000) / 1000;
                f.raf_samples = times.length;
                // Matrix / scorer aliases (H08)
                f.raf_cv = f.raf_jitter_cv;
              }
              if (f.perf_now_resolution_ms != null) {
                f.clock_resolution_ms = f.perf_now_resolution_ms;
              }
              f.data_ok = f.perf_now_resolution_ms != null || f.raf_jitter_cv != null;
              enqueue(ctx, "B25_clock_raf", f, 51, "main");
            }
          }
          requestAnimationFrame(tick);
          return;
        }
      } catch (e2) {}
      if (f.perf_now_resolution_ms != null) {
        f.clock_resolution_ms = f.perf_now_resolution_ms;
      }
      f.data_ok = f.perf_now_resolution_ms != null;
      enqueue(ctx, "B25_clock_raf", f, 51, "main");
    },
  });

  /**
   * H03 — shader numeric / precision / ULP spectrum (claim vs behavior).
   * Multi-point mediump/highp divergence + digest (not raw pixel dump).
   */
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

  /**
   * H05 — caps claim vs actual allocation pressure (anti camoufox caps).
   * v2: denser staircase + renderbuffer pressure until fail (bounded).
   */
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


  /**
   * B28 — silent readonly peripheral shape (NO permission prompts).
   * Never: getUserMedia, geolocation, clipboard, Notification.requestPermission,
   * camera/mic/geo permissions.query. May: Notification.permission read,
   * enumerateDevices counts (no labels), privacy_guard passive site media.
   */
  register("B28_permissions_media", {
    priority: 50,
    schedule: "dynamic",
    batch_id: "B28_permissions_media",
    layer: "deep",
    run: function (ctx) {
      var f = {
        peri_algo: "gr_permissions_readonly_v2",
        privacy_policy: "silent_no_permission_request",
        collected_at: Date.now(),
      };
      try {
        if (global.GRPrivacyGuard && global.GRPrivacyGuard.flushFields) {
          var pg = global.GRPrivacyGuard.flushFields();
          Object.keys(pg).forEach(function (k) {
            f[k] = pg[k];
          });
        }
      } catch (ePg) {}
      try {
        if (typeof Notification !== "undefined" && Notification.permission != null) {
          f.permissions_notifications = String(Notification.permission);
        } else {
          f.permissions_notifications = "unknown";
        }
      } catch (eN) {
        f.permissions_notifications = "error";
      }
      f.permissions_geolocation = "not_probed";
      f.geolocation_permission = "not_probed";
      f.permissions_camera = "not_probed";
      f.permissions_microphone = "not_probed";
      f.permissions_clipboard = "not_probed";
      f.permissions_matrix = {
        notifications: f.permissions_notifications,
        geolocation: "not_probed",
        camera: "not_probed",
        microphone: "not_probed",
        "clipboard-read": "not_probed",
        "clipboard-write": "not_probed",
      };
      f.permission_states =
        "notifications=" +
        f.permissions_notifications +
        "|geolocation=not_probed|camera=not_probed|microphone=not_probed|clipboard=not_probed";
      f.permission_shape_digest = simpleHash(f.permission_states).slice(0, 12);
      f.permissions_hash = f.permission_shape_digest;
      f.permissions_granted_n = f.permissions_notifications === "granted" ? 1 : 0;
      f.permissions_denied_n = f.permissions_notifications === "denied" ? 1 : 0;
      f.permissions_prompt_n = f.permissions_notifications === "default" ? 1 : 0;
      f.no_permission_request = true;
      function finishMedia(list) {
        var kinds = { audioinput: 0, audiooutput: 0, videoinput: 0, other: 0 };
        (list || []).forEach(function (d) {
          var k = (d && d.kind) || "other";
          if (kinds[k] == null) kinds.other++;
          else kinds[k]++;
        });
        f.media_devices_count = (list || []).length;
        f.media_devices_kinds = kinds;
        f.media_input_count = kinds.audioinput || 0;
        f.media_output_count = kinds.audiooutput || 0;
        f.media_video_count = kinds.videoinput || 0;
        f.media_devices_enumerate = true;
        f.media_labels_collected = false;
        f.data_ok = true;
        enqueue(ctx, "B28_permissions_media", f, 50, "main");
      }
      try {
        if (navigator.mediaDevices && navigator.mediaDevices.enumerateDevices) {
          navigator.mediaDevices
            .enumerateDevices()
            .then(function (list) {
              finishMedia(list);
            })
            .catch(function () {
              f.media_devices_enumerate = false;
              f.data_ok = true;
              enqueue(ctx, "B28_permissions_media", f, 50, "main");
            });
          return;
        }
      } catch (eM) {}
      f.media_devices_enumerate = false;
      f.data_ok = true;
      enqueue(ctx, "B28_permissions_media", f, 50, "main");
    },
  });

  /**
   * B29 — Sensors / battery hedge (D13/H12).
   * iss/18 P0: capture one-shot orientation + accel vectors when permitted.
   */
  register("B29_sensors_battery", {
    priority: 49,
    schedule: "dynamic",
    batch_id: "B29_sensors_battery",
    layer: "deep",
    run: function (ctx) {
      var f = {
        sensors_algo: "gr_sensors_battery_v2",
        collected_at: Date.now(),
      };
      f.sensor_accel_present = typeof Accelerometer !== "undefined";
      f.sensor_gyro_present = typeof Gyroscope !== "undefined";
      f.sensor_orient_present = typeof AbsoluteOrientationSensor !== "undefined";
      // Presence-only (no addEventListener) — listening triggers Firefox "sensor deprecated" spam.
      f.device_motion = typeof DeviceMotionEvent !== "undefined";
      f.device_orientation_api = typeof DeviceOrientationEvent !== "undefined";
      f.max_touch_points = navigator.maxTouchPoints != null ? navigator.maxTouchPoints : null;
      f.device_motion_sample_ok = false;
      f.sensors_listen_skipped = true;
      var pending = 1;
      function tick() {
        pending--;
        if (pending > 0) return;
        f.data_ok =
          f.battery_level != null ||
          f.sensor_accel_present ||
          f.device_motion ||
          f.device_orientation_api ||
          (f.max_touch_points != null && f.max_touch_points > 0);
        enqueue(ctx, "B29_sensors_battery", f, 49, "main");
      }
      try {
        if (navigator.getBattery) {
          pending++;
          navigator.getBattery().then(function (b) {
            f.battery_level = b.level;
            f.battery_charging = b.charging;
            f.battery_charging_time = b.chargingTime;
            f.battery_discharging_time = b.dischargingTime;
            tick();
          }).catch(function () {
            f.battery_error = "getBattery_rejected";
            tick();
          });
        }
      } catch (eB) {
        f.battery_error = String((eB && eB.message) || eB);
      }
      tick();
    },
  });

  /**
   * B30 — GPU bandwidth ladder deepen (H02): upload/readback/roundtrip.
   */
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

  /**
   * B34 — CPU cache / working-set ladder (H07).
   */
  register("B84_gpu_bandwidth_ladder", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B84_gpu_bandwidth_ladder",
    layer: "hard",
    run: function (ctx) {
      // Reuse B30 implementation via forced re-run with pack id tag
      var f = {
        bw_algo: "gr_a4_bandwidth_ladder_v1",
        pohw_direction: "A4",
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
          enqueue(ctx, "B84_gpu_bandwidth_ladder", f, 54, "main");
          return;
        }
        var sizes = [64, 128, 256, 512, 1024, 2048];
        var readback = [];
        sizes.forEach(function (sz) {
          c.width = sz;
          c.height = sz;
          gl.viewport(0, 0, sz, sz);
          gl.clearColor(0.05, 0.1, 0.15, 1);
          gl.clear(gl.COLOR_BUFFER_BIT);
          var out = new Uint8Array(sz * sz * 4);
          var t1 = performance.now();
          gl.readPixels(0, 0, sz, sz, gl.RGBA, gl.UNSIGNED_BYTE, out);
          gl.finish();
          var rbMs = performance.now() - t1;
          var bytes = sz * sz * 4;
          readback.push({
            size: sz,
            wall_ms: Math.round(rbMs * 1000) / 1000,
            mib_s: rbMs > 0 ? Math.round((bytes / (rbMs / 1000) / (1024 * 1024)) * 100) / 100 : null,
          });
        });
        f.gpu_readback_ladder = readback;
        f.gpu_bandwidth_ladder = readback;
        if (readback.length >= 3) {
          var xs = readback.map(function (r) { return r.size; });
          var ys = readback.map(function (r) { return r.wall_ms; });
          var n = xs.length, sx = 0, sy = 0, sxx = 0, sxy = 0, i;
          for (i = 0; i < n; i++) {
            sx += xs[i]; sy += ys[i]; sxx += xs[i] * xs[i]; sxy += xs[i] * ys[i];
          }
          var den = n * sxx - sx * sx;
          f.gpu_wall_staircase_slope = den ? (n * sxy - sx * sy) / den : null;
          f.roundtrip_slope = f.gpu_wall_staircase_slope;
        }
        f.data_ok = readback.length >= 4;
      } catch (e) {
        f.bw_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B84_gpu_bandwidth_ladder", f, 54, "main");
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


  /**
   * B26 — Agent/global symbol parity (v57 collectAgentParity).
   * Presence matrix of automation/engine globals for br kernel hedge.
   */
  register("B26_agent_parity", {
    priority: 47,
    schedule: "dynamic",
    batch_id: "B26_agent_parity",
    layer: "deep",
    run: function (ctx) {
      var f = {
        agent_parity_algo: "gr_agent_parity_v1",
        collected_at: Date.now(),
      };
      var keys = [
        "chrome",
        "safari",
        "opera",
        "InstallTrigger",
        "callPhantom",
        "_phantom",
        "__nightmare",
        "__selenium_unwrapped",
        "__webdriver_evaluate",
        "__driver_evaluate",
        "__webdriver_script_fn",
        "__puppeteer_evaluation_script__",
        "__chromedriver_evaluate",
        "domAutomation",
        "domAutomationController",
        "cdc_adoQpoasnfa76pfcZLmcfl_Array",
        "BrowserAutomationToolkit",
        "webdriver",
        "spawn",
        "emit",
        "Buffer",
        "process",
        "require",
      ];
      var root = typeof window !== "undefined" ? window : self;
      var present = {};
      var hit = 0;
      keys.forEach(function (k) {
        var ok = false;
        try {
          // hasOwnProperty only — never typeof InstallTrigger / root[k] (deprecated binding).
          if (k === "InstallTrigger") {
            ok = Object.prototype.hasOwnProperty.call(root, "InstallTrigger");
          } else {
            ok = Object.prototype.hasOwnProperty.call(root, k);
            if (!ok) {
              try {
                ok = typeof root[k] !== "undefined";
              } catch (eT) {
                ok = false;
              }
            }
          }
        } catch (e) {
          ok = false;
        }
        // also navigator.webdriver
        if (k === "webdriver") {
          try {
            ok = ok || !!(navigator && navigator.webdriver);
          } catch (e2) {}
        }
        present[k] = ok;
        if (ok) hit++;
      });
      f.agent_parity_keys_n = keys.length;
      f.agent_parity_hit_n = hit;
      f.agent_parity_matrix = present;
      f.agent_parity_hash = simpleHash(
        keys.map(function (k) { return present[k] ? "1" : "0"; }).join("")
      ).slice(0, 12);
      // Flatten high-signal automation globals for scorers (beyond bare hash)
      f.agent_has_webdriver = !!present.webdriver;
      f.agent_has_phantom = !!(present.callPhantom || present._phantom);
      f.agent_has_selenium = !!(present.__selenium_unwrapped || present.__webdriver_evaluate || present.__driver_evaluate);
      f.agent_has_puppeteer = !!(present.__puppeteer_evaluation_script__ || present.__chromedriver_evaluate);
      f.agent_has_cdc = !!present.cdc_adoQpoasnfa76pfcZLmcfl_Array;
      f.agent_has_dom_automation = !!(present.domAutomation || present.domAutomationController);
      f.agent_parity_hit_ratio =
        keys.length > 0 ? Math.round((hit / keys.length) * 1000) / 1000 : 0;
      f.agent_automation_globals_n = [
        "callPhantom",
        "_phantom",
        "__nightmare",
        "__selenium_unwrapped",
        "__webdriver_evaluate",
        "__puppeteer_evaluation_script__",
        "__chromedriver_evaluate",
        "domAutomation",
      ].filter(function (k) { return present[k]; }).length;
      f.data_ok = true;
      enqueue(ctx, "B26_agent_parity", f, 47, "main");
    },
  });

  /**
   * B27 — Storage / privacy surface deep (v57 storage + D09/D36 lite).
   */
  register("B27_storage_privacy", {
    priority: 46,
    schedule: "dynamic",
    batch_id: "B27_storage_privacy",
    layer: "deep",
    run: function (ctx) {
      var f = {
        storage_algo: "gr_storage_privacy_v1",
        collected_at: Date.now(),
      };
      try {
        f.local_storage = (function () {
          try {
            var k = "__gr_ls__";
            localStorage.setItem(k, "1");
            localStorage.removeItem(k);
            return true;
          } catch (e) {
            return false;
          }
        })();
        f.session_storage = (function () {
          try {
            var k = "__gr_ss__";
            sessionStorage.setItem(k, "1");
            sessionStorage.removeItem(k);
            return true;
          } catch (e) {
            return false;
          }
        })();
      } catch (eS) {}
      f.indexedDB = typeof indexedDB !== "undefined";
      f.caches_api = typeof caches !== "undefined";
      f.cookie_enabled = navigator.cookieEnabled != null ? !!navigator.cookieEnabled : null;
      try {
        f.cookie_count = document.cookie ? document.cookie.split(";").filter(Boolean).length : 0;
      } catch (eC) {
        f.cookie_count = null;
      }
      f.openDatabase = typeof openDatabase !== "undefined";
      f.service_worker = !!(navigator.serviceWorker);
      f.storage_manager = !!(navigator.storage && navigator.storage.estimate);
      function finish() {
        f.privacy_storage_score =
          (f.local_storage ? 1 : 0) +
          (f.session_storage ? 1 : 0) +
          (f.indexedDB ? 1 : 0) +
          (f.caches_api ? 1 : 0) +
          (f.cookie_enabled ? 1 : 0) +
          (f.service_worker ? 1 : 0);
        f.data_ok = true;
        enqueue(ctx, "B27_storage_privacy", f, 46, "main");
      }
      try {
        if (navigator.storage && navigator.storage.estimate) {
          navigator.storage.estimate().then(function (est) {
            f.storage_quota = est && est.quota != null ? est.quota : null;
            f.storage_usage = est && est.usage != null ? est.usage : null;
            if (navigator.storage.persisted) {
              return navigator.storage.persisted().then(function (p) {
                f.storage_persisted = !!p;
                finish();
              });
            }
            finish();
          }).catch(function () {
            finish();
          });
          return;
        }
      } catch (eE) {}
      finish();
    },
  });

  /**
   * B35 — DomRect layout + Performance timeline digests (v57 DomRect/PerfTimeline).
   */
  register("B35_dom_perf", {
    priority: 45,
    schedule: "dynamic",
    batch_id: "B35_dom_perf",
    layer: "deep",
    run: function (ctx) {
      var f = {
        dom_perf_algo: "gr_dom_perf_v1",
        collected_at: Date.now(),
      };
      // DomRect probe
      try {
        if (typeof document !== "undefined" && document.body) {
          var d = document.createElement("div");
          d.style.cssText =
            "position:absolute;left:-9999px;top:0;width:100.5px;height:33.3px;font:16px Arial;padding:0;margin:0;";
          d.textContent = "mmmmmmmmmmlli";
          document.body.appendChild(d);
          var r = d.getBoundingClientRect();
          f.dom_rect = {
            width: Math.round(r.width * 1000) / 1000,
            height: Math.round(r.height * 1000) / 1000,
            x: Math.round(r.x * 1000) / 1000,
            y: Math.round(r.y * 1000) / 1000,
          };
          f.dom_rect_hash = simpleHash(
            [f.dom_rect.width, f.dom_rect.height, f.dom_rect.x, f.dom_rect.y].join(",")
          ).slice(0, 12);
          // subpixel detection
          f.dom_rect_subpixel = Math.abs(r.width - Math.round(r.width)) > 1e-6;
          document.body.removeChild(d);
        } else {
          f.dom_rect_skip = "no_document";
        }
      } catch (eD) {
        f.dom_rect_error = String((eD && eD.message) || eD);
      }
      // Performance timing surface
      try {
        f.perf_time_origin = performance.timeOrigin || null;
        f.perf_now = performance.now();
        if (performance.timing) {
          var t = performance.timing;
          f.perf_nav_timing = {
            dom_complete_ms: t.domComplete && t.navigationStart ? t.domComplete - t.navigationStart : null,
            load_event_ms: t.loadEventEnd && t.navigationStart ? t.loadEventEnd - t.navigationStart : null,
            response_start_ms:
              t.responseStart && t.navigationStart ? t.responseStart - t.navigationStart : null,
          };
        }
        // PerformanceObserver entry counts (snapshot)
        if (performance.getEntriesByType) {
          var types = ["navigation", "resource", "paint", "measure", "mark"];
          f.perf_entry_counts = {};
          var supported =
            typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes
              ? PerformanceObserver.supportedEntryTypes
              : null;
          types.forEach(function (ty) {
            try {
              if (supported && supported.indexOf(ty) < 0) {
                f.perf_entry_counts[ty] = null;
                return;
              }
              f.perf_entry_counts[ty] = (performance.getEntriesByType(ty) || []).length;
            } catch (e) {
              f.perf_entry_counts[ty] = null;
            }
          });
          f.perf_timeline_hash = simpleHash(JSON.stringify(f.perf_entry_counts)).slice(0, 12);
        }
        // long task support
        f.perf_longtask_observer =
          typeof PerformanceObserver !== "undefined" &&
          (function () {
            try {
              return PerformanceObserver.supportedEntryTypes
                ? PerformanceObserver.supportedEntryTypes.indexOf("longtask") >= 0
                : false;
            } catch (e) {
              return false;
            }
          })();
      } catch (eP) {
        f.perf_error = String((eP && eP.message) || eP);
      }
      f.data_ok = !!(f.dom_rect_hash || f.perf_timeline_hash || f.perf_nav_timing);
      enqueue(ctx, "B35_dom_perf", f, 45, "main");
    },
  });


  /**
   * B36 — H04 raster / MSAA / edge rule digests (implementation family, not SKU id).
   */
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

  /**
   * B37 — H09 thermal/frequency drift LITE (short multi-burst, not 60–120s).
   */
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

  /**
   * B38 — H15 negative/spoof dictionary surface aggregation (FE observables only).
   */
  register("B38_neg_dict", {
    priority: 43,
    schedule: "dynamic",
    batch_id: "B38_neg_dict",
    layer: "deep",
    run: function (ctx) {
      var f = {
        neg_dict_algo: "gr_h15_neg_dict_v1",
        pohw_direction: "H15",
        collected_at: Date.now(),
      };
      var hits = [];
      function hit(code, ok) {
        if (ok) hits.push(code);
      }
      try {
        hit("webdriver", !!navigator.webdriver);
        hit("outer_zero", window.outerWidth === 0 && window.outerHeight === 0);
        hit("plugins_empty", !navigator.plugins || navigator.plugins.length === 0);
        hit("languages_empty", !navigator.languages || navigator.languages.length === 0);
        hit("chrome_missing", typeof window.chrome === "undefined" && /Chrome\//.test(navigator.userAgent || ""));
        hit("permission_denied_all", false); // filled if permissions API bulk-denied later
        hit("callPhantom", !!(window.callPhantom || window._phantom));
        hit("selenium", !!(window.__selenium_unwrapped || window._Selenium_IDE_Recorder));
        hit("puppeteer", !!(window.__puppeteer_evaluation_script__));
        hit("cdc_prop", Object.keys(window).some(function (k) { return k.indexOf("cdc_") === 0; }));
        hit("headless_ua", /HeadlessChrome|PhantomJS|Electron/i.test(navigator.userAgent || ""));
        hit("webgl_soft_label", (function () {
          try {
            var c = document.createElement("canvas");
            var gl = c.getContext("webgl") || c.getContext("experimental-webgl");
            if (!gl) return true;
            var dbg = gl.getExtension("WEBGL_debug_renderer_info");
            var r = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : "";
            return /swiftshader|llvmpipe|softpipe|basic render/i.test(String(r));
          } catch (e) {
            return false;
          }
        })());
      } catch (eN) {
        f.neg_error = String((eN && eN.message) || eN);
      }
      f.neg_dict_hits = hits;
      f.neg_dict_hit_n = hits.length;
      f.neg_dict_hash = simpleHash(hits.sort().join("|")).slice(0, 12);
      f.data_ok = true;
      enqueue(ctx, "B38_neg_dict", f, 43, "main");
    },
  });

  /**
   * B39 — D08 memory / heap pressure ladder (device tier).
   */
  register("B39_mem_pressure", {
    priority: 42,
    schedule: "dynamic",
    batch_id: "B39_mem_pressure",
    layer: "deep",
    run: function (ctx) {
      var f = {
        mem_algo: "gr_d08_mem_pressure_v1",
        collected_at: Date.now(),
        device_memory: navigator.deviceMemory != null ? navigator.deviceMemory : null,
        hardware_concurrency: navigator.hardwareConcurrency != null ? navigator.hardwareConcurrency : null,
      };
      try {
        if (performance && performance.memory) {
          f.js_heap_size_limit = performance.memory.jsHeapSizeLimit;
          f.total_js_heap_size = performance.memory.totalJSHeapSize;
          f.used_js_heap_size = performance.memory.usedJSHeapSize;
        }
      } catch (eM) {}
      // allocation ladder until soft fail
      var sizes = [1, 2, 4, 8, 16, 32]; // MB attempts (small)
      var ladder = [];
      var maxOk = 0;
      for (var i = 0; i < sizes.length; i++) {
        var mb = sizes[i];
        try {
          var n = (mb * 1024 * 1024) / 4;
          var t0 = performance.now();
          var buf = new Float32Array(n);
          buf[0] = 1;
          buf[buf.length - 1] = 2;
          var ms = performance.now() - t0;
          ladder.push({ mb: mb, ok: true, alloc_ms: Math.round(ms * 1000) / 1000 });
          maxOk = mb;
          buf = null;
        } catch (eA) {
          ladder.push({ mb: mb, ok: false, err: String((eA && eA.message) || eA) });
          break;
        }
      }
      f.mem_alloc_ladder = ladder;
      f.mem_alloc_max_mb = maxOk;
      f.data_ok = ladder.length > 0;
      enqueue(ctx, "B39_mem_pressure", f, 42, "main");
    },
  });

  /**
   * B40 — D21 WebSocket fingerprint (stack timing / constructor surface).
   */
  register("B40_websocket_fp", {
    priority: 41,
    schedule: "dynamic",
    batch_id: "B40_websocket_fp",
    layer: "deep",
    run: function (ctx) {
      var f = {
        ws_algo: "gr_d21_websocket_fp_v1",
        collected_at: Date.now(),
        websocket_present: typeof WebSocket !== "undefined",
      };
      if (typeof WebSocket === "undefined") {
        f.data_ok = false;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
        return;
      }
      try {
        // Constructor fingerprint only — no live /gr-ws-probe (nginx returns 200, floods console).
        f.ws_constructor_native = /\[native code\]/.test(Function.prototype.toString.call(WebSocket));
        f.ws_binary_types = null;
        try {
          f.ws_proto_keys_n = Object.getOwnPropertyNames(WebSocket.prototype || {}).length;
        } catch (ePk) {
          f.ws_proto_keys_n = null;
        }
        f.ws_url_host = typeof location !== "undefined" ? location.host : null;
        f.ws_probe_mode = "constructor_only";
        f.ws_connect_skipped = "no_ws_upgrade_endpoint";
        f.ws_close_reason = "skipped_no_endpoint";
        f.data_ok = true;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
      } catch (e) {
        f.ws_error = String((e && e.message) || e);
        f.data_ok = true;
        enqueue(ctx, "B40_websocket_fp", f, 41, "main");
      }
    },
  });

  /**
   * B41 — D35 HID / gamepad presence (permission-safe).
   */
  register("B41_hid_gamepad", {
    priority: 40,
    schedule: "dynamic",
    batch_id: "B41_hid_gamepad",
    layer: "deep",
    run: function (ctx) {
      var f = {
        hid_algo: "gr_d35_hid_gamepad_v1",
        collected_at: Date.now(),
        hid_api: !!(navigator.hid),
        usb_api: !!(navigator.usb),
        serial_api: !!(navigator.serial),
        bluetooth_api: !!(navigator.bluetooth),
        gamepad_api: typeof navigator.getGamepads === "function",
      };
      try {
        if (f.gamepad_api) {
          var gps = navigator.getGamepads() || [];
          var n = 0;
          var ids = [];
          for (var i = 0; i < gps.length; i++) {
            if (gps[i]) {
              n++;
              ids.push(String(gps[i].id || "").slice(0, 64));
            }
          }
          f.gamepad_count = n;
          f.gamepad_ids_sample = ids.slice(0, 4);
        }
      } catch (eG) {
        f.gamepad_error = String((eG && eG.message) || eG);
      }
      f.hid_surface_score =
        (f.hid_api ? 1 : 0) +
        (f.usb_api ? 1 : 0) +
        (f.serial_api ? 1 : 0) +
        (f.bluetooth_api ? 1 : 0) +
        (f.gamepad_api ? 1 : 0);
      f.data_ok = true;
      enqueue(ctx, "B41_hid_gamepad", f, 40, "main");
    },
  });


  /**
   * B42 — H09 full thermal/frequency drift (progressive long probe).
   * Lite B37 = fast probe; this pack continues progressive phases A→B→C.
   * Budget: ctx.thermal_full_max_ms or __GR_THERMAL_FULL_MS__ (default 45000, max 120000).
   * Async between phases so the page stays responsive; each phase emits richer slope.
   */
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

  /**
   * B43 — Error / engine stack fingerprint (v57 collectErrors).
   */
  register("B43_errors_engine", {
    priority: 39,
    schedule: "dynamic",
    batch_id: "B43_errors_engine",
    layer: "deep",
    run: function (ctx) {
      var f = {
        errors_algo: "gr_errors_engine_v1",
        collected_at: Date.now(),
      };
      function catchShape(fn) {
        try {
          fn();
          return { threw: false };
        } catch (e) {
          return {
            threw: true,
            name: e && e.name ? String(e.name) : "Error",
            message: e && e.message ? String(e.message).slice(0, 120) : "",
            stack_head: e && e.stack ? String(e.stack).split("\\n").slice(0, 3).join("|").slice(0, 200) : "",
          };
        }
      }
      f.errors_engine = {
        null_prop: catchShape(function () { return null.x; }),
        undef_call: catchShape(function () { return undefined(); }),
        object_call: catchShape(function () { return ({})(); }),
        new_number: catchShape(function () { return new (1)(); }),
      };
      // Engine-specific strings
      var msgs = Object.keys(f.errors_engine).map(function (k) {
        var e = f.errors_engine[k];
        return e.threw ? e.name + ":" + e.message : "ok";
      });
      f.errors_engine_hash = simpleHash(msgs.join("|")).slice(0, 12);
      f.errors_engine_chrome_like = msgs.some(function (m) {
        return /Cannot read propert|is not a function|undefined/.test(m);
      });
      f.data_ok = true;
      enqueue(ctx, "B43_errors_engine", f, 39, "main");
    },
  });

  /**
   * B44 — Speech voices deep (v57 speech / D14).
   */
  register("B44_speech_deep", {
    priority: 37,
    schedule: "dynamic",
    batch_id: "B44_speech_deep",
    layer: "deep",
    run: function (ctx) {
      var f = {
        speech_algo: "gr_speech_deep_v1",
        collected_at: Date.now(),
      };
      function packVoices(vs) {
        f.speech_voices_count = vs.length;
        f.speech_langs = [];
        var sample = [];
        var seen = {};
        vs.forEach(function (v) {
          var lang = v.lang || "";
          if (lang && !seen[lang]) {
            seen[lang] = 1;
            f.speech_langs.push(lang);
          }
          if (sample.length < 24) {
            sample.push({
              name: String(v.name || "").slice(0, 48),
              lang: lang,
              local: !!v.localService,
              default: !!v.default,
            });
          }
        });
        f.speech_voices_sample = sample;
        f.speech_voices_hash = simpleHash(
          vs.map(function (v) { return (v.name || "") + "|" + (v.lang || ""); }).join(";")
        ).slice(0, 12);
        f.data_ok = vs.length >= 0;
        enqueue(ctx, "B44_speech_deep", f, 37, "main");
      }
      try {
        if (!window.speechSynthesis) {
          f.speech_skip = "no_speechSynthesis";
          f.speech_voices_count = 0;
          f.data_ok = true;
          enqueue(ctx, "B44_speech_deep", f, 37, "main");
          return;
        }
        var vs = speechSynthesis.getVoices() || [];
        if (vs.length) {
          packVoices(vs);
          return;
        }
        // chrome loads async
        var done = false;
        speechSynthesis.onvoiceschanged = function () {
          if (done) return;
          done = true;
          packVoices(speechSynthesis.getVoices() || []);
        };
        setTimeout(function () {
          if (done) return;
          done = true;
          packVoices(speechSynthesis.getVoices() || []);
        }, 400);
      } catch (e) {
        f.speech_error = String((e && e.message) || e);
        f.data_ok = false;
        enqueue(ctx, "B44_speech_deep", f, 37, "main");
      }
    },
  });

  /**
   * B45 — H11 display / HDR / screen physical digests.
   */
  register("B45_display_hdr", {
    priority: 36,
    schedule: "dynamic",
    batch_id: "B45_display_hdr",
    layer: "deep",
    run: function (ctx) {
      var f = {
        display_algo: "gr_h11_display_hdr_v1",
        pohw_direction: "H11",
        collected_at: Date.now(),
      };
      try {
        var smH11 = readScreenMetrics();
        f.device_pixel_ratio = window.devicePixelRatio || null;
        f.screen_width = smH11.screen_width;
        f.screen_height = smH11.screen_height;
        f.screen_avail_width = smH11.screen_avail_width;
        f.screen_avail_height = smH11.screen_avail_height;
        f.screen_fp_protection_suspect = smH11.screen_fp_protection_suspect;
        f.screen_color_depth = smH11.color_depth;
        f.screen_pixel_depth = smH11.pixel_depth;
        f.inner_width = window.innerWidth;
        f.outer_width = window.outerWidth;
        try {
          f.orientation_type = screen.orientation && screen.orientation.type;
          f.orientation_angle = screen.orientation && screen.orientation.angle;
        } catch (eO) {}
        // matchMedia display capabilities
        var mqs = [
          "(color-gamut: srgb)",
          "(color-gamut: p3)",
          "(color-gamut: rec2020)",
          "(dynamic-range: high)",
          "(video-dynamic-range: high)",
          "(prefers-contrast: more)",
          "(prefers-reduced-transparency: reduce)",
          "(update: fast)",
          "(hover: hover)",
          "(pointer: fine)",
          "(any-pointer: coarse)",
        ];
        f.display_mq = {};
        mqs.forEach(function (q) {
          try {
            f.display_mq[q] = window.matchMedia(q).matches;
          } catch (e) {
            f.display_mq[q] = null;
          }
        });
        f.display_mq_hash = simpleHash(
          mqs.map(function (q) { return f.display_mq[q] ? "1" : "0"; }).join("")
        ).slice(0, 12);
        f.css_color_gamut = f.display_mq["(color-gamut: p3)"]
          ? "p3"
          : f.display_mq["(color-gamut: rec2020)"]
            ? "rec2020"
            : f.display_mq["(color-gamut: srgb)"]
              ? "srgb"
              : "unknown";
        f.hdr_likely = !!(f.display_mq["(dynamic-range: high)"] || f.display_mq["(video-dynamic-range: high)"]);
        f.data_ok = true;
      } catch (e) {
        f.display_error = String((e && e.message) || e);
        f.data_ok = false;
      }
      enqueue(ctx, "B45_display_hdr", f, 36, "main");
    },
  });

  /**
   * B46 — D03 audio deep digests.
   * Multi-seed OfflineAudio compressor+biquad → moments / peak bins / phase digest.
   */
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

  /**
   * B47 — SAB/Atomics high-resolution clock foundation (iss/45 A3 · iss/54 P8).
   * Resource class: **cpu** — may run parallel with gpu (B10/B18) and audio (B46).
   * Not a silicon UV claim: calibrates tick rate + event-loop jitter for tz/timing extras.
   * Honest skip when !crossOriginIsolated (need COOP/COEP).
   */
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

  // iss/58 A3/A5/A10 packs B81-B83
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



  

  
  /* =====================================================================
   * Phase-1 field-depth expansion packs (B86–B91), see
   * docs/design/iss/Green V6 探测字段深度利用与多源探测扩展综合战略.md §3.1
   * All packs: honest skip fields, no permission surface, budget-capped.
   * ===================================================================== */

  /** B86: OS/Gecko surface — Firefox version facets, Linux distro hints, coarse OS vote. */
  register("B86_os_gecko_surface", {
    priority: 45, schedule: "dynamic", batch_id: "B86_os_gecko_surface", layer: "mid",
    run: function(ctx){
      var f={os_probe_algo:"gr_os_gecko_surface_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B86_os_gecko_surface",f,45,"main");return f;}
      try{f.oscpu_raw=(typeof navigator!=="undefined"&&navigator.oscpu!=null)?String(navigator.oscpu):null;}catch(e){f.oscpu_raw=null;}
      try{f.build_id_raw=(typeof navigator!=="undefined"&&navigator.buildID!=null)?String(navigator.buildID):null;}catch(e){f.build_id_raw=null;}
      try{f.productsub_raw=(typeof navigator!=="undefined"&&navigator.productSub!=null)?String(navigator.productSub):null;}catch(e){f.productsub_raw=null;}
      try{if("mozInnerScreenX" in window){f.gecko_moz_innerscreen_x_delta=Math.round((window.mozInnerScreenX-(window.screenX||0))*100)/100;}else{f.gecko_moz_innerscreen_x_delta=null;}}catch(e){f.gecko_moz_innerscreen_x_delta=null;}
      function widthOf(fam){try{var c=document.createElement("canvas");c.width=400;c.height=48;var g=c.getContext("2d");g.font="32px "+fam;return g.measureText("mmMwWLliI0O&1").width;}catch(e){return -1;}}
      var wBase=widthOf("monospace"),wA=widthOf("Arimo, monospace"),wC=widthOf("Cousine, monospace"),wT=widthOf("Tinos, monospace");
      f.linux_metric_font_arimo=(wBase>0&&wA>0)?(Math.abs(wA-wBase)>0.5):null;
      f.linux_metric_font_cousine=(wBase>0&&wC>0)?(Math.abs(wC-wBase)>0.5):null;
      f.linux_metric_font_tinos=(wBase>0&&wT>0)?(Math.abs(wT-wBase)>0.5):null;
      var fam=null,vh=null;
      try{
        var ua=String(navigator.userAgent||""),pl=String(navigator.platform||"");
        var uad=(typeof navigator!=="undefined"&&navigator.userAgentData)?navigator.userAgentData:null;
        var plat=uad&&uad.platform?String(uad.platform).toLowerCase():"";
        if(/android/i.test(ua)||plat.indexOf("android")===0){fam="android";}
        else if(/iphone|ipad|ipod/i.test(ua)||(/mac/i.test(pl)&&/mobile/i.test(ua))){fam="ios";}
        else if(/windows/i.test(ua)||/win/i.test(pl)||plat.indexOf("windows")===0){fam="windows";}
        else if(/mac os\s*x|macintosh|mac/i.test(ua)){fam="macos";}
        else if(/cr os|chrome os/i.test(ua)||plat.indexOf("chrome os")===0){fam="chromeos";}
        else if(/linux/i.test(ua)||/linux/i.test(pl)){fam="linux";}
        else{fam="unknown";}
        if(fam==="windows"&&uad&&uad.platformVersion){
          var pv=String(uad.platformVersion||"").split("."),maj=pv[0]||"0",sub=pv[2]||"0";
          if(maj==="6"&&sub==="1")vh="windows_7";else if(maj==="6"&&sub==="2")vh="windows_8";
          else if(maj==="6"&&sub==="3")vh="windows_8_1";else if(maj==="10"&&parseInt(sub,10)>=22000)vh="windows_11";
          else if(maj==="10")vh="windows_10";
        }
        if(fam==="macos"&&uad&&uad.platformVersion){
          var mv=String(uad.platformVersion||"").replace(/_/g,"."),mm=parseFloat(mv);
          vh=mm>=13?"macos_13_plus":mm>=12?"macos_12":mm>=11?"macos_11":"macos_10_15_old";
        }
      }catch(e){}
      f.os_kernel_hint=fam;f.os_version_hint_raw=vh;
      return Promise.resolve(done());
    }
  });

  /** B87: engine behavior difference suite — error texts, stack shape, math roundoff, farbling delta. */
  register("B87_engine_behavior_diff", {
    priority: 43, schedule: "dynamic", batch_id: "B87_engine_behavior_diff", layer: "mid",
    run: function(ctx){
      var f={eb_probe_algo:"gr_engine_behavior_diff_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B87_engine_behavior_diff",f,43,"main");return f;}
      var errs=[];
      function tryErr(thunk){try{thunk();}catch(e){errs.push(String(e&&e.message||e));}}
      tryErr(function(){new Function("alert()");});
      tryErr(function(){new Function("1 2");});
      tryErr(function(){[].sort.call();});
      tryErr(function(){null[0]();});
      tryErr(function(){(1).toString(37);});
      tryErr(function(){new (null)();});
      tryErr(function(){({}).safeCall();});
      tryErr(function(){decodeURIComponent("%");});
      tryErr(function(){new Function("var x = ;");});
      f.engine_error_9set_digest=simpleHash16(errs.join("|")).slice(0,12);
      try{var st=new Error("gr_eb");f.engine_stack_first_line=String((st.stack||"").split("\n")[0]||"").slice(0,80);}catch(e){f.engine_stack_first_line="";}
      try{f.engine_stack_trace_limit=(typeof Error!=="undefined"&&Error.stackTraceLimit!=null)?Number(Error.stackTraceLimit):null;}catch(e){f.engine_stack_trace_limit=null;}
      try{
        var rp=[Math.cos(1e20),Math.sin(1e20),Math.tan(1e21),Math.pow(2e-3,-100),Math.hypot(1e200,1e200),Math.exp(1e3)];
        f.engine_math_roundoff_profile=rp.map(function(v){if(!isFinite(v))return v>0?"inf":(v<0?"ninf":"nan");return v.toPrecision(6);});
        f.engine_math_roundoff_digest=simpleHash16(f.engine_math_roundoff_profile.join(",")).slice(0,12);
      }catch(e){f.engine_math_roundoff_profile=null;}
      try{
        function drawHash(){var c=document.createElement("canvas");c.width=96;c.height=48;var g=c.getContext("2d");
          g.fillStyle="#1a2b3c";g.fillRect(0,0,96,48);
          for(var i=0;i<6;i++){g.beginPath();g.arc(12+i*14,18+(i%2)*10,6,0,Math.PI*2);g.fillStyle="rgb("+((i*47)%255)+","+((i*91)%255)+",180)";g.fill();}
          g.fillStyle="#fff";g.font="14px sans-serif";g.fillText("gr|"+i+"|"+Math.PI,4,40);
          try{return simpleHash(c.toDataURL()).slice(0,12);}catch(e){return null;}}
        var h1=drawHash(),h2=drawHash();
        f.engine_canvas_farbling_delta=(h1!=null&&h2!=null)?(h1===h2?0:1):null;
      }catch(e){f.engine_canvas_farbling_delta=null;}
      try{
        var ua=String(navigator.userAgent||""),uad=navigator.userAgentData;
        f.engine_realm_ua_align_ok=null;f.engine_realm_grease_present=null;
        if(uad){
          var mChrome=/Chrome\/([0-9]+)/.exec(ua),uaV=mChrome?mChrome[1]:null,uaFull=uad.uaFullVersion?String(uad.uaFullVersion):null,brands=uad.brands||[];
          f.engine_realm_ua_align_ok=(uaFull!=null&&uaV!=null)?Math.abs(parseInt(uaFull,10)-parseInt(uaV,10))<=1:null;
          f.engine_realm_grease_present=brands.some(function(b){var n=String(b.brand||"");return /Not[A-Z]\/?A?\)?Brand|Not\/A\)Brand|Not_A_Brand/.test(n)||n.length>=8;});
        }
      }catch(e){}
      return Promise.resolve(done());
    }
  });

  /** B88: wasm instruction-level throughput — CPU microarch C-key (not silicon V). */
  register("B88_wasm_instruction_throughput", {
    priority: 52, schedule: "dynamic", batch_id: "B88_wasm_instruction_throughput", layer: "hard",
    run: function(ctx){
      var f={wt_probe_algo:"gr_wasm_instruction_throughput_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B88_wasm_instruction_throughput",f,52,"main");return f;}
      if(typeof WebAssembly==="undefined"||typeof WebAssembly.Module!=="function"){f.wasm_bi_simd_skip="no_wasm";return Promise.resolve(done());}
      var SCALAR=[0,97,115,109,1,0,0,0,1,31,6,96,1,127,1,125,96,1,127,1,124,96,1,127,1,125,96,1,127,1,126,96,1,127,1,127,96,1,127,1,126,2,49,4,3,101,110,118,4,103,70,51,50,3,125,1,3,101,110,118,4,103,70,54,52,3,124,1,3,101,110,118,4,103,73,51,50,3,127,1,3,101,110,118,4,103,73,54,52,3,126,1,3,7,6,0,1,2,3,4,5,7,57,6,6,102,51,50,109,117,108,0,0,6,102,54,52,109,117,108,0,1,6,102,51,50,100,105,118,0,2,6,105,54,52,100,105,118,0,3,6,105,51,50,109,117,108,0,4,8,102,54,52,116,111,105,54,52,0,5,10,244,1,6,40,2,1,127,1,125,2,64,3,64,32,1,32,0,79,13,1,67,0,0,128,63,32,2,35,0,148,32,1,65,1,106,33,1,12,0,11,11,32,2,11,44,2,1,127,1,124,2,64,3,64,32,1,32,0,79,13,1,68,0,0,0,0,0,0,248,63,32,2,35,1,162,32,1,65,1,106,33,1,12,0,11,11,32,2,11,40,2,1,127,1,125,2,64,3,64,32,1,32,0,79,13,1,67,0,0,224,63,32,2,35,0,149,32,1,65,1,106,33,1,12,0,11,11,32,2,11,38,2,1,127,1,126,2,64,3,64,32,1,32,0,79,13,1,66,200,1,32,2,35,3,128,32,1,65,1,106,33,1,12,0,11,11,32,2,11,37,2,1,127,1,127,2,64,3,64,32,1,32,0,79,13,1,65,19,32,2,35,2,108,32,1,65,1,106,33,1,12,0,11,11,32,2,11,38,2,1,127,1,126,2,64,3,64,32,1,32,0,79,13,1,66,0,35,1,177,32,2,124,32,1,65,1,106,33,1,12,0,11,11,32,2,11];
      var SIMD=[0,97,115,109,1,0,0,0,1,11,2,96,1,127,1,125,96,1,127,1,124,2,49,4,3,101,110,118,4,103,70,51,50,3,125,1,3,101,110,118,4,103,70,54,52,3,124,1,3,101,110,118,4,103,73,51,50,3,127,1,3,101,110,118,4,103,73,54,52,3,126,1,3,3,2,0,1,7,23,2,8,102,51,50,120,52,109,117,108,0,0,8,102,54,52,120,50,109,117,108,0,1,10,105,2,49,2,1,127,1,123,2,64,3,64,32,1,32,0,79,13,1,67,0,0,128,63,253,19,32,2,35,0,253,19,253,230,1,32,1,65,1,106,33,1,12,0,11,11,32,2,253,31,0,11,53,2,1,127,1,123,2,64,3,64,32,1,32,0,79,13,1,68,0,0,0,0,0,0,248,63,253,20,32,2,35,1,253,20,253,242,1,32,1,65,1,106,33,1,12,0,11,11,32,2,253,33,0,11];
      try{
        var t0=performance.now();var mod=new WebAssembly.Module(new Uint8Array(SCALAR));f.wasm_compile_ms=Math.round((performance.now()-t0)*100)/100;
      }catch(e){f.wasm_bi_simd_skip="scalar_compile_failed";return Promise.resolve(done());}
      function b88Imports(){try{var i64v=(typeof BigInt!=="undefined")?7n:BigInt(7);return {env:{gF32:new WebAssembly.Global({value:"f32",mutable:true},1.7),gF64:new WebAssembly.Global({value:"f64",mutable:true},2.718281828459045),gI32:new WebAssembly.Global({value:"i32",mutable:true},31),gI64:new WebAssembly.Global({value:"i64",mutable:true},i64v)}};}catch(e){return {};}}
      var inst=null;
      try{var t1=performance.now();inst=new WebAssembly.Instance(mod,b88Imports());f.wasm_instantiate_ms=Math.round((performance.now()-t1)*100)/100;}
      catch(e){f.wasm_bi_simd_skip="instantiate_failed";return Promise.resolve(done());}
      var ex=inst.exports,N=3000000,REPS=3,wall0=Date.now();
      function median(ms){var a=ms.slice().sort(function(x,y){return x-y;});return a[Math.floor(a.length/2)];}
      function bench(fn){var runs=[];for(var r=0;r<REPS;r++){var t=performance.now();fn(N);runs.push((performance.now()-t)*1e6/N);}return Math.round(median(runs)*100)/100;}
      f.wasm_bi_f32_mul_ns=bench(ex.f32mul);
      f.wasm_bi_f64_mul_ns=bench(ex.f64mul);
      f.wasm_bi_f32_div_ns=bench(ex.f32div);
      if(Date.now()-wall0>2400){f.wasm_bi_partial=true;}
      f.wasm_bi_i64_div_ns=bench(ex.i64div);
      f.wasm_bi_i32_mul_ns=bench(ex.i32mul);
      f.wasm_trunc_ftoi_ns=bench(ex.f64toi64);
      try{
        if(WebAssembly.validate(new Uint8Array(SIMD))){
          var sm=new WebAssembly.Module(new Uint8Array(SIMD)),si=new WebAssembly.Instance(sm,b88Imports());
          f.wasm_simd_f32x4_ns=bench(si.exports.f32x4mul);
          f.wasm_simd_f64x2_ns=bench(si.exports.f64x2mul);
        }else{f.wasm_bi_simd_skip="simd_unavailable";}
      }catch(e){f.wasm_bi_simd_skip="simd_compile_failed";}
      if(f.wasm_simd_f32x4_ns){f.wasm_scalar_vs_simd_ratio=Math.round(f.wasm_bi_f32_mul_ns/Math.max(0.01,f.wasm_simd_f32x4_ns)*100)/100;}
      f.wasm_throughput_digest=simpleHash16([f.wasm_bi_f32_mul_ns,f.wasm_bi_f64_mul_ns,f.wasm_bi_f32_div_ns,f.wasm_bi_i64_div_ns,f.wasm_bi_i32_mul_ns,f.wasm_trunc_ftoi_ns,f.wasm_simd_f32x4_ns,f.wasm_simd_f64x2_ns,f.wasm_scalar_vs_simd_ratio].join(",")).slice(0,12);
      return Promise.resolve(done());
    }
  });

  /** B89: audio known-lock — fake-DSP detection fodder (automation axis, never device_id). */
  register("B89_audio_known_lock", {
    priority: 46, schedule: "dynamic", batch_id: "B89_audio_known_lock", layer: "hard",
    run: function(ctx){
      var f={al_probe_algo:"gr_audio_known_lock_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B89_audio_known_lock",f,46,"main");return f;}
      if(typeof OfflineAudioContext==="undefined"){f.audio_known_lock_skip="no_offline_audio";return Promise.resolve(done());}
      var LEN=5000,SR=44100;
      function render(oscFreq,withComp){
        return new Promise(function(resolve){
          try{
            var ctx=new OfflineAudioContext(1,LEN,SR);
            var osc=ctx.createOscillator();osc.type="triangle";osc.frequency.value=oscFreq;
            var gain=ctx.createGain();gain.gain.value=0.9;
            var comp=null,analyser=null;
            osc.connect(gain);
            if(withComp){comp=ctx.createDynamicsCompressor();comp.threshold.value=-50;comp.knee.value=40;comp.ratio.value=12;comp.attack.value=0;comp.release.value=0.25;gain.connect(comp);}
            analyser=ctx.createAnalyser();analyser.fftSize=256;
            var tail=withComp?comp:gain;tail.connect(analyser);analyser.connect(ctx.destination);
            osc.start(0);
            var ctxT0=Date.now();
            ctx.startRendering().then(function(buf){
              var ch=buf.getChannelData(0),sum=0,i,cdiff=0,freqSum=0,timeSum=0;
              for(i=4500;i<5000;i++){sum+=Math.abs(ch[i]);}
              var copy=new Float32Array(LEN);try{buf.copyFromChannel(copy,0);}catch(e){}
              for(i=0;i<LEN;i++){cdiff+=Math.abs(ch[i]-copy[i]);}
              var freq=new Float32Array(analyser.frequencyBinCount),td=new Float32Array(analyser.fftSize);
              try{analyser.getFloatFrequencyData(freq);analyser.getFloatTimeDomainData(td);}catch(e){}
              for(i=0;i<freq.length;i++){freqSum+=freq[i];}
              for(i=0;i<td.length;i++){timeSum+=Math.abs(td[i]);}
              resolve({sum:sum,cdiff:cdiff/LEN,freqSum:freqSum,timeSum:timeSum,ms:Date.now()-ctxT0});
            }).catch(function(){resolve(null);});
          }catch(e){resolve(null);}
        });
      }
      return Promise.all([render(10000,true),render(0,false)]).then(function(rs){
        var main=rs[0],sil=rs[1];
        if(!main){f.audio_known_lock_skip="render_failed";return done();}
        f.audio_known_lock_sum=Math.round(main.sum*1e4)/1e4;
        f.audio_channel_vs_copy_delta=Math.round(main.cdiff*10000)/10000;
        f.audio_float_freq_sum=Math.round(main.freqSum*100)/100;
        f.audio_float_time_sum=Math.round(main.timeSum*1000)/1000;
        f.audio_render_ms=main.ms;
        if(sil){
          f.audio_silent_osc_unique_bins=(sil.freqSum>-120)||(sil.sum>0)?1:0;
        }else{f.audio_silent_osc_unique_bins=null;}
        f.audio_known_lock_digest=simpleHash16([f.audio_known_lock_sum,f.audio_channel_vs_copy_delta,f.audio_silent_osc_unique_bins].join("|")).slice(0,12);
        return done();
      });
    }
  });

  /** B90: OS emoji raster + mac dot-font measurability + fallback-chain digest. */
  register("B90_os_emoji_raster", {
    priority: 41, schedule: "dynamic", batch_id: "B90_os_emoji_raster", layer: "mid",
    run: function(ctx){
      var f={er_probe_algo:"gr_os_emoji_raster_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B90_os_emoji_raster",f,41,"main");return f;}
      try{
        var EMO=["\uD83D\uDE00","\uD83D\uDE0D","\uD83E\uDD16","\uD83E\uDD84","\uD83D\uDE80","\uD83C\uDF0D","\uD83C\uDF89","\uD83C\uDF4E","\uD83C\uDFC6","\uD83D\uDC8E","\uD83D\uDD76\uFE0F","\uD83C\uDFAF","\uD83E\uDDE0","\u26A1","\uD83D\uDD25","\uD83C\uDF08","\uD83C\uDF5C","\uD83C\uDFD4\uFE0F","\uD83C\uDFAE","\uD83D\uDC7E"];
        var rows=4,cols=5,cell=64,w=cols*cell,h=rows*cell;
        var c=document.createElement("canvas");c.width=w;c.height=h;
        var g=c.getContext("2d");g.fillStyle="#fff";g.fillRect(0,0,w,h);
        g.font="44px serif";g.textAlign="center";g.textBaseline="middle";
        for(var i=0;i<EMO.length;i++){var x=(i%cols)*cell+cell/2,y=Math.floor(i/cols)*cell+cell/2;g.fillText(EMO[i],x,y);}
        f.emoji_raster_hash=simpleHash16(c.toDataURL()).slice(0,12);
        var img=g.getImageData(0,0,w,h).data,tofu=0;
        for(var ci=0;ci<EMO.length;ci++){
          var cx=(ci%cols)*cell,cy=Math.floor(ci/cols)*cell,pix=0;
          for(var py=cy+4;py<cy+cell-4;py+=4){for(var px=cx+4;px<cx+cell-4;px+=4){var o=(py*w+px)*4;if(img[o+3]>40)pix++;}}
          if(pix<6)tofu++;
        }
        f.emoji_tofu_count=tofu;
      }catch(e){f.emoji_skip="render_failed";}
      try{
        var mw=function(fam){var c2=document.createElement("canvas");c2.width=400;c2.height=48;var g2=c2.getContext("2d");g2.font="32px "+fam;return g2.measureText("mmMwWLliI0O&1").width;};
        var s=mw("-apple-system, BlinkMacSystemFont"),b=mw("Segoe UI"),r=mw("Roboto, Noto Sans"),n=mw("Noto Sans"),a=mw("Arial"),mo=mw("monospace");
        f.font_fallback_chain_digest=simpleHash16([s,b,r,n,a,mo].join(",")).slice(0,12);
        var dot=false;
        try{if(document.fonts&&document.fonts.check){dot=document.fonts.check('32px ".SFNS-Regular"')||document.fonts.check('32px ".AppleSystemUIFont"');}}catch(eD){}
        f.mac_dot_font_measurable=!!dot;
      }catch(e){}
      return Promise.resolve(done());
    }
  });

  /** B91: storage disk quota — VM/container/browser-isolation clues + cross-session stability token. */
  register("B91_storage_disk_quota", {
    priority: 38, schedule: "dynamic", batch_id: "B91_storage_disk_quota", layer: "mid",
    run: function(ctx){
      var f={sq_probe_algo:"gr_storage_disk_quota_v1",collected_at:Date.now()};
      function done(){f.data_ok=true;enqueue(ctx,"B91_storage_disk_quota",f,38,"main");return f;}
      function gridCls(q){var GB=1073741824,TB=1099511627776;if(q>=4*TB)return "multi_tb";if(q>=TB)return "tb";if(q>=512*GB)return "512g_1t";if(q>=128*GB)return "128g_512g";if(q>=32*GB)return "32g_128g";return "lt_32g";}
      if(!navigator.storage||!navigator.storage.estimate){f.storage_quota_skip="no_estimate";return Promise.resolve(done());}
      return navigator.storage.estimate().then(function(est){
        var q=Number(est&&est.quota)||0,u=Number(est&&est.usage)||0,GB=1073741824;
        f.storage_quota_bytes=q;f.storage_usage_bytes=u;
        f.storage_quota_grid_class=gridCls(q);
        f.storage_quota_round_suspect=(q%GB===0)||(q%(GB*10)===0);
        var cls=f.storage_quota_grid_class,prev=null;
        try{prev=localStorage.getItem("gr_sq_cls");}catch(e){}
        f.storage_quota_prev_seen=prev!=null;
        f.storage_quota_prev_match=(prev!=null)&&(prev===cls);
        try{localStorage.setItem("gr_sq_cls",cls);}catch(e){}
        return done();
      }).catch(function(){f.storage_quota_skip="estimate_failed";return done();});
    }
  });

  // ── iss/74 Phase 2 research packs (B92–B95) ───────────────────────────────
  // B92/B94/B95 are research-gated server-side (component_catalog gate=research):
  // FE still implements and may run when the brain unlocks them (B5 contradiction
  // rule) or the dual-KPI gate clears; B93 is default mid (no new permissions).

  register("B92_webgpu_atomic_contention", {
    priority: 55, schedule: "dynamic", batch_id: "B92_webgpu_atomic_contention", layer: "hard",
    run: function (ctx) {
      var f = { atomic_contention_algo: "gr_webgpu_atomic_contention_v1", collected_at: Date.now() };
      function done() { f.data_ok = true; enqueue(ctx, "B92_webgpu_atomic_contention", f, 55, "main"); return f; }
      function skip(k) { f.atomic_contention_skip = k; f.atomic_contention_ok = false; f.data_ok = false; enqueue(ctx, "B92_webgpu_atomic_contention", f, 55, "main"); return f; }
      if (!navigator.gpu || !navigator.gpu.requestAdapter) return Promise.resolve(skip("no_webgpu"));
      // Seeded anti-replay: server challenge seed when available, else session nonce.
      f.atomic_contention_seed_k = (ctx && ctx.challenge_seed)
        ? simpleHash16(String(ctx.challenge_seed))
        : simpleHash16(String(Date.now()) + "|" + String(Math.floor(Math.random() * 1e9)));
      var presets = [
        { wg: 4, loop: 256, rounds: 6 },   // short preset ≤300ms on normal GPUs
        { wg: 16, loop: 1024, rounds: 5 },
        { wg: 48, loop: 4096, rounds: 4 },
      ];
      var SHORT_MS = 300, BUDGET_MS = 2000;
      var t0 = Date.now();
      return navigator.gpu.requestAdapter().then(function (adapter) {
        if (!adapter) return skip("no_adapter");
        return adapter.requestDevice().then(function (device) {
          var size = 4 + 4096 * 4;
          var storage = device.createBuffer({ size: size, usage: 8 | 4, mappedAtCreation: false }); // STORAGE|COPY_SRC
          var staging = device.createBuffer({ size: size, usage: 8 | 2, mappedAtCreation: false }); // COPY_SRC|COPY_DST
          var code =
            "struct Buf { counter: atomic<u32>, data: array<u32> };\n" +
            "@group(0) @binding(0) var<storage, read_write> buf: Buf;\n" +
            "@compute @workgroup_size(64)\n" +
            "fn main(@builtin(global_invocation_id) gid: vec3<u32>) {\n" +
            "  for (var i = 0u; i < LOOP; i++) {\n" +
            "    if ((i ^ gid.x) % 2u == 0u) { atomicAdd(&buf.counter, 1u); }\n" +
            "  }\n" +
            "}";
          function moduleFor(loop) { return device.createShaderModule({ code: code.replace("LOOP", String(loop)) }); }
          var bgl = device.createBindGroupLayout({ entries: [{ binding: 0, visibility: 4, buffer: { type: "storage" } }] }); // COMPUTE
          var pl = device.createPipelineLayout({ bindGroupLayouts: [bgl] });
          var bg = device.createBindGroup({ layout: bgl, entries: [{ binding: 0, resource: { buffer: storage } }] });
          var roundsAll = [], chi = new Array(16).fill(0), presetsDone = 0;
          function runPreset(wg, loop, rounds) {
            if (Date.now() - t0 > BUDGET_MS) return Promise.resolve(false);
            var pipeline = device.createComputePipeline({ layout: pl, compute: { module: moduleFor(loop), entryPoint: "main" } });
            var doneRounds = 0;
            function oneRound() {
              if (Date.now() - t0 > BUDGET_MS) return Promise.resolve(true);
              device.queue.writeBuffer(storage, 0, new Uint32Array([0]));
              var enc = device.createCommandEncoder();
              var pass = enc.beginComputePass();
              pass.setPipeline(pipeline);
              pass.setBindGroup(0, bg);
              pass.dispatchWorkgroups(wg);
              pass.end();
              enc.copyBufferToBuffer(storage, 0, staging, 0, 4);
              var tS = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
              device.queue.submit([enc.finish()]);
              if (!device.queue.onSubmittedWorkDone) return Promise.resolve(false);
              return device.queue.onSubmittedWorkDone().then(function () {
                var d = ((typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now()) - tS;
                roundsAll.push(d);
                doneRounds++;
                if (doneRounds < rounds) return oneRound();
                return Promise.resolve(true);
              });
            }
            return oneRound();
          }
          var chain = Promise.resolve();
          presets.forEach(function (p) {
            chain = chain.then(function () {
              return runPreset(p.wg, p.loop, p.rounds).then(function (ok) {
                if (ok) presetsDone++;
              });
            });
          });
          return chain.then(function () {
            device.destroy();
            if (roundsAll.length < 2) return skip("too_few_rounds");
            var sorted = roundsAll.slice().sort(function (a, b) { return a - b; });
            var min = sorted[0], max = sorted[sorted.length - 1], span = (max - min) || 1;
            sorted.forEach(function (d) { chi[Math.min(15, Math.floor(((d - min) / span) * 16))]++; });
            var med = sorted[Math.floor(sorted.length / 2)], ent = 0;
            chi.forEach(function (c) { if (c > 0) { var p0 = c / sorted.length; ent -= p0 * Math.log2(p0); } });
            f.atomic_contention_hist16 = chi;
            f.atomic_contention_median_ms = Math.round(med * 1000) / 1000;
            f.atomic_contention_entropy = Math.round(ent * 10000) / 10000;
            f.atomic_contention_digest = simpleHash16(roundsAll.map(function (d) { return Math.round(d * 1000); }).join(","));
            f.atomic_contention_presets_n = presetsDone;
            f.atomic_contention_rounds_n = roundsAll.length;
            f.atomic_contention_wall_ms = Date.now() - t0;
            f.atomic_contention_ok = true;
            f.atomic_contention_quality = (typeof document !== "undefined" && document.hidden) ? "throttled" : "ok";
            return done();
          });
        }).catch(function (e) { return skip("webgpu_error:" + String((e && e.message) || e)); });
      }).catch(function (e) { return skip("webgpu_error:" + String((e && e.message) || e)); });
    }
  });

  register("B93_blink_fork_matrix", {
    priority: 44, schedule: "dynamic", batch_id: "B93_blink_fork_matrix", layer: "mid",
    run: function (ctx) {
      var f = { fork_matrix_algo: "gr_blink_fork_matrix_v1", collected_at: Date.now() };
      var ua = String(navigator.userAgent || "");
      var brands = null;
      try {
        if (navigator.userAgentData && navigator.userAgentData.brands && navigator.userAgentData.brands.length) {
          brands = navigator.userAgentData.brands.map(function (b) { return b.brand + ":" + b.version; });
        }
      } catch (e) {}
      f.fork_matrix_brand_order = brands || [];
      f.fork_matrix_brand_n = (brands || []).length;
      var greaseN = 0, greasePos = [];
      function isGreaseBrand(bn) {
        // No case-insensitive flag here: /i would flip [^a-z] to also exclude
        // uppercase letters, so "Not A(Brand)" would never match.
        return /CRBrand/i.test(bn) || /Not[^a-z]*Brand|^Not[A-Za-z_]+$/.test(bn);
      }
      (brands || []).forEach(function (b, i) {
        var bn = b.split(":")[0] || "";
        if (isGreaseBrand(bn)) { greaseN++; greasePos.push(i); }
      });
      f.fork_matrix_grease_n = greaseN;
      f.fork_matrix_grease_positions = greasePos;
      f.fork_matrix_ua_has_edg = /Edg\//.test(ua);
      f.fork_matrix_ua_has_opr = /OPR\//.test(ua);
      var edgeBrand = brands ? brands.some(function (b) { return /microsoft edge/i.test(b.split(":")[0]); }) : false;
      f.fork_matrix_edge_brand = edgeBrand;
      // UA major vs first non-grease brand major (version/order check).
      var uaM = null, m = /(?:Chrome|Chromium)\/(\d+)/.exec(ua);
      if (m) uaM = m[1];
      var brandM = null;
      if (brands) {
        for (var bi = 0; bi < brands.length; bi++) {
          var bs = brands[bi];
          var bv = bs.split(":")[1] || "";
          var bm = /^(\d+)/.exec(bv);
          // GREASE entries carry the 99.x spoof major — skip them per UA-CH spec.
          if (bm && !isGreaseBrand(bs.split(":")[0] || "")) { brandM = bm[1]; break; }
        }
      }
      f.fork_matrix_brand_ua_major_match = (uaM !== null && brandM !== null) ? (uaM === brandM) : null;
      // Static CSS var fork surface (no permissions; best-effort).
      var known = ["--brave", "--arc-app-primary", "--arc", "--floorp", "--waterfox", "--mises", "--vivaldi"];
      var found = [];
      try {
        var cs = getComputedStyle(document.documentElement);
        known.forEach(function (v) { if (cs.getPropertyValue(v)) found.push(v); });
      } catch (e) {}
      f.fork_matrix_css_vars = found;
      var guess = "unknown";
      if (f.fork_matrix_ua_has_edg && edgeBrand) guess = "edge";
      else if (f.fork_matrix_ua_has_opr) guess = "opera";
      else if (found.indexOf("--brave") >= 0) guess = "brave";
      else if (found.indexOf("--arc-app-primary") >= 0 || found.indexOf("--arc") >= 0) guess = "arc";
      else if (found.indexOf("--floorp") >= 0) guess = "floorp";
      else if (found.indexOf("--waterfox") >= 0) guess = "waterfox";
      else if (/Firefox\//.test(ua)) guess = "firefox";
      else if (/Safari\//.test(ua) && !/Chrome\//.test(ua)) guess = "safari";
      else if (/Chrome\//.test(ua) || /Chromium\//.test(ua)) guess = "chrome";
      f.blink_fork_guess = guess;
      var uaFam = /Edg\//.test(ua) ? "edge" : (/OPR\//.test(ua) ? "opera" : (/Firefox\//.test(ua) ? "firefox"
        : (/Safari\//.test(ua) && !/Chrome\//.test(ua) ? "safari" : (/Chrome\//.test(ua) || /Chromium\//.test(ua) ? "chrome" : "unknown"))));
      var cssFam = found.indexOf("--brave") >= 0 ? "brave"
        : ((found.indexOf("--arc-app-primary") >= 0 || found.indexOf("--arc") >= 0) ? "arc"
          : (found.indexOf("--floorp") >= 0 ? "floorp" : (found.indexOf("--waterfox") >= 0 ? "waterfox" : "")));
      var cssOk = !found.length || cssFam === "" || cssFam === uaFam
        || (cssFam === "brave" && uaFam === "chrome") || (cssFam === "arc" && uaFam === "chrome")
        || (cssFam === "floorp" && uaFam === "firefox") || (cssFam === "waterfox" && uaFam === "firefox");
      f.fork_claim_vs_obs = found.length ? (cssOk ? "agree" : "mismatch") : "unknown";
      f.data_ok = true;
      enqueue(ctx, "B93_blink_fork_matrix", f, 44, "main");
      return f;
    }
  });

  register("B94_sab_dual_clock_differential", {
    priority: 53, schedule: "dynamic", batch_id: "B94_sab_dual_clock_differential", layer: "mid",
    run: function (ctx) {
      var f = {
        sab_dual_clock_algo: "gr_sab_dual_clock_differential_v1",
        collected_at: Date.now(),
        cross_origin_isolated: typeof crossOriginIsolated !== "undefined" ? !!crossOriginIsolated : false,
        has_shared_array_buffer: typeof SharedArrayBuffer !== "undefined",
        has_atomics: typeof Atomics !== "undefined",
      };
      function done(ok) { f.data_ok = ok !== false; enqueue(ctx, "B94_sab_dual_clock_differential", f, 53, "main"); return f; }
      if (!f.cross_origin_isolated || !f.has_shared_array_buffer || !f.has_atomics) {
        f.sab_dual_clock_skip = "need_coop_coep_cross_origin_isolated";
        f.sab_dual_clock_ok = false;
        return Promise.resolve(done(true));
      }
      // SAB calibration anchor + quantized-clock differential (Fantastic-Timers-style
      // minimal-delta recovery for performance.now / Date.now; rAF when present).
      try {
        var sab = new SharedArrayBuffer(8);
        var i32 = new Int32Array(sab);
        Atomics.store(i32, 0, 0);
        Atomics.store(i32, 1, 1);
        f.sab_dual_clock_anchor = (Atomics.load(i32, 0) === 0 && Atomics.load(i32, 1) === 1);
      } catch (e) {
        f.sab_dual_clock_skip = "sab_failed"; f.sab_dual_clock_ok = false;
        return Promise.resolve(done(true));
      }
      var minPerf = 0, prev = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
      var tEnd = Date.now() + 8;
      while (Date.now() < tEnd) {
        var n = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
        var d = n - prev;
        if (d > 0 && (minPerf === 0 || d < minPerf)) minPerf = d;
        prev = n;
      }
      var minDate = 0, p2 = Date.now(), tEnd2 = Date.now() + 8;
      while (Date.now() < tEnd2) {
        var d2 = Date.now() - p2;
        if (d2 > 0 && (minDate === 0 || d2 < minDate)) minDate = d2;
        p2 = Date.now();
      }
      f.sab_dual_clock_perf_quant_us = Math.round(minPerf * 1000) / 1000;
      f.sab_dual_clock_date_quant_ms = minDate;
      f.sab_dual_clock_digest = simpleHash16(String(minPerf.toFixed(4)) + "|" + String(minDate));
      f.sab_dual_clock_ok = true;
      if (typeof requestAnimationFrame !== "function") return Promise.resolve(done(true));
      return new Promise(function (res) {
        var frames = [], tPrev = null, nFrames = 0, deadline = Date.now() + 700;
        function tick(ts) {
          if (tPrev !== null && nFrames <= 6) { var df = ts - tPrev; if (df > 0) frames.push(df); }
          tPrev = ts;
          nFrames++;
          if (nFrames < 7 && Date.now() < deadline) { requestAnimationFrame(tick); return; }
          var m = frames.length ? Math.min.apply(null, frames) : 0;
          f.sab_dual_clock_raf_quant_ms = m ? Math.round(m * 1000) / 1000 : 0;
          res(done(true));
        }
        requestAnimationFrame(tick);
      });
    }
  });

  register("B95_gpu_eu_timing", {
    priority: 50, schedule: "dynamic", batch_id: "B95_gpu_eu_timing", layer: "hard",
    run: function (ctx) {
      var f = { gpu_eu_algo: "gr_gpu_eu_timing_drawn_apart_v1", collected_at: Date.now() };
      function done() { f.data_ok = true; enqueue(ctx, "B95_gpu_eu_timing", f, 50, "main"); return f; }
      function skip(k) { f.gpu_eu_skip = k; f.gpu_eu_ok = false; f.data_ok = false; enqueue(ctx, "B95_gpu_eu_timing", f, 50, "main"); return f; }
      f.gpu_eu_engine_gate = detectEngineFamily();
      if (f.gpu_eu_engine_gate !== "blink") return Promise.resolve(skip("engine_gate_not_blink"));
      if (typeof OffscreenCanvas === "undefined") return Promise.resolve(skip("no_offscreen_canvas"));
      var subsets = [256, 512, 1024, 2048];
      var progress = [false, false, false, false], acc = [];
      try {
        var raw = localStorage.getItem("gr_eu_subset_v1");
        if (raw) { var st = JSON.parse(raw); if (st && st.done && st.curve) { progress = st.done; acc = st.curve; } }
      } catch (e) {}
      var idx = -1, i;
      for (i = 0; i < subsets.length; i++) { if (!progress[i]) { idx = i; break; } }
      if (idx < 0) {
        f.gpu_eu_curve = acc;
        f.gpu_eu_accum_done_n = acc.length;
        f.gpu_eu_digest = simpleHash16(acc.join(","));
        f.gpu_eu_ok = true;
        f.gpu_eu_partial = false;
        try { localStorage.removeItem("gr_eu_subset_v1"); } catch (e) {}
        return Promise.resolve(done());
      }
      var BUDGET_MS = 2000, tA = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
      var canvas = new OffscreenCanvas(64, 64);
      var gl = canvas.getContext("webgl2");
      f.gpu_eu_subset_i = idx;
      f.gpu_eu_subset_n = subsets.length;
      if (!gl) return Promise.resolve(skip("webgl2_unavailable"));
      var src =
        "#version 300 es\n" +
        "in vec2 a;\nout float v;\n" +
        "void main() {\n" +
        "  float x = a.x + 0.001;\n" +
        "  for (int k = 0; k < " + subsets[idx] + "; k++) { x = sinh(x); }\n" +
        "  v = x;\n" +
        "  gl_Position = vec4(a, 0.0, 1.0);\n" +
        "}";
      var vs = gl.createShader(gl.VERTEX_SHADER);
      gl.shaderSource(vs, src);
      gl.compileShader(vs);
      var fs = gl.createShader(gl.FRAGMENT_SHADER);
      gl.shaderSource(fs, "void main(){ gl_FragColor = vec4(0.5); }");
      gl.compileShader(fs);
      var prog = gl.createProgram();
      gl.attachShader(prog, vs);
      gl.attachShader(prog, fs);
      gl.linkProgram(prog);
      gl.useProgram(prog);
      var buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 3,-1, -1,3, 1,1, 1,-1, -1,1, 1,3]), gl.STATIC_DRAW);
      var loc = gl.getAttribLocation(prog, "a");
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
      var roundMs = [], rounds = 8;
      for (var r = 0; r < rounds; r++) {
        var tS = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
        for (var d = 0; d < 12; d++) gl.drawArrays(gl.TRIANGLE_STRIP, 0, 7);
        if (gl.finish) gl.finish();
        roundMs.push(((typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now()) - tS);
        if (Date.now() - tA > BUDGET_MS) break;
      }
      roundMs.forEach(function (v) { acc.push(Math.round(v * 1000) / 1000); });
      progress[idx] = roundMs.length >= 2;
      try {
        localStorage.setItem("gr_eu_subset_v1", JSON.stringify({ done: progress, curve: acc }));
      } catch (e) {}
      f.gpu_eu_round_ms = roundMs;
      f.gpu_eu_curve = acc;
      f.gpu_eu_accum_done_n = acc.length;
      f.gpu_eu_partial = true;
      f.gpu_eu_ok = true;
      f.gpu_eu_wall_ms = Date.now() - tA;
      f.gpu_eu_digest = simpleHash16(acc.join(","));
      return Promise.resolve(done());
    }
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
    cpu_loop_algo_id: "gr_cpu_curve_v2_multiworkload",
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
  try {
    global.__GR_CPU_LOOP_ALGO__ = "gr_cpu_curve_v2_multiworkload";
    global.__GR_LITE_BUILD_ALGO__ = "gr_cpu_curve_v2_multiworkload";
    if (global.GRCollectors && global.GRCollectors.__h) {
      global.GRCollectors.__h.cpu_loop_algo_id = "gr_cpu_curve_v2_multiworkload";
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
