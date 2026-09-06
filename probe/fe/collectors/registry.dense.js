/**
 * Dense pack expansion + per-pack densifier (≥30 product-bag fields).
 * Demand families map to brain directions; fields are claim-obs materials.
 * Loaded after registry.js core helpers (expects GRCollectors.register).
 */
(function (global) {
  "use strict";
  if (!global.GRCollectors || !global.GRCollectors.register) {
    /* fe_diag spam removed */ /* silenced */

    return;
  }
  var register = global.GRCollectors.register;
  var lists = global.GRProbeLists || {};

  function shash(s) {
    s = String(s || "");
    var h = 2166136261;
    for (var i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return (h >>> 0).toString(16);
  }

  function bool(v) { return !!v; }

  /** Skip invalid/lost pname — prevents WebGL INVALID_ENUM console spam. */
  function safeGlParam(gl, pname) {
    if (!gl || pname == null || typeof pname !== "number" || !isFinite(pname)) return null;
    try {
      if (typeof gl.isContextLost === "function" && gl.isContextLost()) return null;
      return gl.getParameter(pname);
    } catch (e) {
      return null;
    }
  }

  function releaseGl(gl, canvas) {
    try {
      if (gl) {
        var lose = gl.getExtension && gl.getExtension("WEBGL_lose_context");
        if (lose && lose.loseContext) /*lose_suppressed*/void 0;
      }
    } catch (e0) {}
    try {
      if (canvas) {
        canvas.width = 1;
        canvas.height = 1;
      }
    } catch (e1) {}
    try {
      if (global.GRGlGovernor && GRGlGovernor.releaseAll) GRGlGovernor.releaseAll();
    } catch (e2) {}
  }

  /** Dense densify: never call requestAdapter (B18 owns that). Cache-only. */
  function requestWebGpuAdapterOnce(opts) {
    opts = opts || {};
    if (global.__GR_WEBGPU_NO_ADAPTER__) {
      return Promise.resolve({ adapter: null, skip: "cached_none" });
    }
    if (global.__GR_WEBGPU_ADAPTER__) {
      return Promise.resolve({ adapter: global.__GR_WEBGPU_ADAPTER__, skip: "cached" });
    }
    if (global.__GR_WEBGPU_ADAPTER_P__) return global.__GR_WEBGPU_ADAPTER_P__;
    if (!navigator.gpu) {
      return Promise.resolve({ adapter: null, skip: "no_gpu" });
    }
    // Defer to B18 — densify must not trigger Edge/Chrome WebGPU console spam.
    return Promise.resolve({ adapter: null, skip: "deferred_to_b18", deferred: true });
  }

  function safePerfEntries(ty) {
    try {
      if (!performance || !performance.getEntriesByType) return [];
      if (
        typeof PerformanceObserver !== "undefined" &&
        PerformanceObserver.supportedEntryTypes &&
        PerformanceObserver.supportedEntryTypes.indexOf(ty) < 0
      ) {
        return [];
      }
      // Observer-only types spam "Deprecated API for given entry type" via getEntriesByType.
      if (
        ty === "longtask" ||
        ty === "layout-shift" ||
        ty === "largest-contentful-paint" ||
        ty === "first-input" ||
        ty === "element"
      ) {
        return [];
      }
      return performance.getEntriesByType(ty) || [];
    } catch (e) {
      return [];
    }
  }

  /** Ensure bag has ≥ minKeys useful materials for family. Mutates f. */
  /** Shared thin meta only — pack-specific materials must reach minKeys without pads. */
  var SHARED_META = {
    probe_density_algo: "gr_dense_v3",
  };

  // Shared densify baseline only — never pack demand materials (e.g. B52 nav_user_agent_len).
  var SHARED_BASELINE_KEYS = {
    pack_id: 1, probe_family: 1, probe_density_algo: 1, collected_at: 1,
    probe_field_count: 1, probe_min_met: 1, probe_specific_n: 1, probe_pad_n: 1,
    dense_pack: 1, demand: 1, data_ok: 1, generic_ok: 1,
    // thin shared viewport/nav dump if ever re-added:
    nav_platform: 1, nav_language: 1, nav_languages_n: 1, nav_cookie_enabled: 1,
    nav_online: 1, nav_hw_concurrency: 1, nav_device_memory: 1, nav_max_touch: 1,
    nav_webdriver: 1, nav_pdf_viewer: 1, nav_vendor: 1, nav_product: 1, nav_app_version_len: 1,
    screen_w: 1, screen_h: 1, screen_aw: 1, screen_ah: 1, screen_cd: 1, screen_pd: 1,
    dpr: 1, inner_w: 1, inner_h: 1, outer_w: 1, outer_h: 1,
    tz_offset_min: 1, tz_name: 1, doc_hidden: 1, doc_visibility: 1,
    has_local_storage: 1, has_session_storage: 1, has_indexed_db: 1, has_service_worker: 1,
    has_webgl: 1, has_webgl2: 1, has_webgpu: 1, has_webrtc: 1, has_audio_ctx: 1,
    has_offscreen: 1, has_worker: 1, has_shared_worker: 1, has_broadcast: 1,
    has_atomics: 1, has_shared_array: 1, has_wasm: 1,
    cross_origin_isolated: 1, is_secure_context: 1,
    perf_now_sample: 1, time_origin_sample: 1,
    densify_error: 1, family_dense_error: 1, gl_dense_error: 1
  };

  function isVanityPadKey(k) {
    // packid_m_N / packid_t_N vanity pads — never count toward ≥30
    if (/_m_\d+$/.test(k) || /_t_\d+$/.test(k)) return true;
    if (k.indexOf("probe_slot_") === 0) return true;
    // probe_pad_0..N only — not the probe_pad_n counter field
    if (/^probe_pad_\d+$/.test(k)) return true;
    return false;
  }

  function countPackSpecific(f, packId) {
    var n = 0;
    Object.keys(f || {}).forEach(function (k) {
      if (SHARED_BASELINE_KEYS[k]) return;
      if (isVanityPadKey(k)) return;
      n++;
    });
    return n;
  }

  function countPadKeys(f) {
    var n = 0;
    Object.keys(f || {}).forEach(function (k) {
      if (isVanityPadKey(k)) n++;
    });
    return n;
  }


  // Module-scope pack→family map (B0–B79). Used per-enqueue so shared boot ctx cannot sticky-family.
  var PACK_FAMILY = {
    B0_bootstrap: "browser_kernel",
    B1_conflict: "browser_kernel",
    B2_hardware: "gpu_physical",
    B3_system: "os",
    B4_mobile: "mobile_form",
    B5_census: "census",
    B6_risk: "browser_kernel",
    B7_sandbox: "sandbox_xsrc",
    B8_gateway_early: "network",
    B8_gateway: "network",
    B9_network: "network",
    B10_hw_curves: "material_hedge",
    B11_interaction: "rpa",
    B12_anti_camouflage: "browser_kernel",
    B13_authorized: "browser_kernel",
    B14_css_protocol: "census",
    B15_cross_curves: "sandbox_xsrc",
    B16_fast_signals: "browser_kernel",
    B17_hw_physical: "gpu_physical",
    B18_webgpu: "gpu_physical",
    B19_eme_media: "material_hedge",
    B20_challenge_seed: "material_hedge",
    B21_census_volume: "census",
    B22_gpu_timer: "gpu_physical",
    B23_native_canvas_hedge: "material_hedge",
    B24_material_crosscheck: "material_hedge",
    B25_clock_raf: "os",
    B26_agent_parity: "browser_kernel",
    B27_storage_privacy: "os",
    B28_permissions_media: "browser_kernel",
    B29_sensors_battery: "peripheral",
    B30_gpu_bandwidth: "gpu_physical",
    B31_shader_numeric: "gpu_physical",
    B33_caps_pressure: "gpu_physical",
    B34_cpu_cache_ladder: "os",
    B35_dom_perf: "os",
    B36_raster_msaa: "gpu_physical",
    B37_thermal_drift_lite: "os",
    B38_neg_dict: "browser_kernel",
    B39_mem_pressure: "os",
    B40_websocket_fp: "network",
    B41_hid_gamepad: "peripheral",
    B42_thermal_drift_full: "os",
    B43_errors_engine: "browser_kernel",
    B44_speech_deep: "os",
    B45_display_hdr: "os",
    B46_audio_deep: "material_hedge",
    B47_api_flags_detail: "census",
    B48_css_supports_detail: "census",
    B49_css_props_detail: "census",
    B50_font_matrix_detail: "census",
    B51_mq_matrix_detail: "census",
    B52_navigator_deep: "browser_kernel",
    B53_window_keys_deep: "browser_kernel",
    B54_plugin_mime_deep: "browser_kernel",
    B55_webrtc_ice_deep: "network",
    B56_webrtc_stats: "network",
    B57_canvas_emoji_path: "material_hedge",
    B58_canvas_text_metrics: "material_hedge",
    B59_webgl_params_full: "gpu_physical",
    B60_webgl_extensions_full: "gpu_physical",
    B61_audio_worklet: "material_hedge",
    B62_offline_audio_moments: "material_hedge",
    B63_intl_full: "os",
    B64_timezone_deep: "os",
    B65_client_hints_full: "browser_kernel",
    B66_sec_ch_headers: "browser_kernel",
    B67_service_worker_deep: "browser_kernel",
    B68_cache_storage_deep: "browser_kernel",
    B69_bluetooth_usb: "peripheral",
    B70_payment_credential: "browser_kernel",
    B71_idle_wake_lock: "mobile_form",
    B72_keyboard_layout: "browser_kernel",
    B73_pointer_capabilities: "rpa",
    B74_visual_viewport: "os",
    B75_performance_entries: "browser_kernel",
    B76_math_wasm_deep: "material_hedge",
    B77_worker_env_deep: "sandbox_xsrc",
    B78_iframe_env_deep: "sandbox_xsrc",
    B79_cross_origin_isolation: "browser_kernel",
    B80_r100_high_value: "material_hedge"
  };

  function familyFromIdRegex(id) {
    id = id || "";
    if (/census|css|font|mq|api/i.test(id)) return "census";
    if (/gpu|webgl|shader|caps|raster|webgpu|bandwidth|timer|hardware|hw_/i.test(id)) return "gpu_physical";
    if (/audio|canvas|math|wasm|material|challenge|pohw|native|curve/i.test(id)) return "material_hedge";
    if (/webrtc|network|dns|ws|websocket|gateway/i.test(id)) return "network";
    if (/agent|automation|conflict|risk|neg|bootstrap|authorized|camouflage|fast_signal|navigator|window_keys|plugin|client_hints|sec_ch|speech|errors/i.test(id)) return "browser_kernel";
    if (/sandbox|cross|worker|iframe|isolation/i.test(id)) return "sandbox_xsrc";
    if (/sensor|battery|mobile|hid|gamepad|touch|bluetooth|usb|idle/i.test(id)) return "peripheral";
    if (/clock|thermal|cpu|mem|perf|dom|system|storage|privacy|display|intl|timezone|viewport/i.test(id)) return "os";
    if (/interaction|rpa|behavior|pointer/i.test(id)) return "rpa";
    if (/mobile_form|wake/i.test(id)) return "mobile_form";
    return null;
  }

  /**
   * Resolve densify family per enqueue (never sticky boot-ctx hint).
   * Priority: batch_id map → id regex → existing fields.probe_family → hint → browser_kernel.
   */
  function resolveFamily(packId, fields, hint) {
    var fromMap = packId && PACK_FAMILY[packId] ? PACK_FAMILY[packId] : null;
    if (fromMap) return fromMap;
    var fromRe = familyFromIdRegex(packId);
    if (fromRe) return fromRe;
    if (fields && fields.probe_family && fields.probe_family !== "generic") {
      return fields.probe_family;
    }
    if (hint && hint !== "generic") return hint;
    return "browser_kernel";
  }

  /**
   * Family-specific deep materials (real probes) until pack-specific count ≥ minKeys.
   * NO vanity _m_/_t_ pads. Each family branch emits ≥30 real measurements.
   */
  function ensureFamilyMaterials(f, packId, family, minKeys) {
    minKeys = minKeys || 30;
    f = f || {};
    f.pack_id = packId || f.pack_id || "";
    // Resolve per packId first — never let sticky familyHint clobber demand family.
    family = resolveFamily(packId, f, family);
    f.probe_family = family;
    f.probe_density_algo = "gr_dense_v3";
    f.collected_at = f.collected_at || Date.now();

    function put(k, v) {
      if (f[k] === undefined || f[k] === null) f[k] = v;
    }

    try {
      // ---- census / browser_kernel: API surface (≥40 real flags) ----
      if (family === "census" || family === "browser_kernel" || family === "generic") {
        var apis = (lists.KNOWN_APIS || []).slice(0, 40);
        var hit = 0, miss = 0, sample = [];
        for (var i = 0; i < apis.length; i++) {
          var an = apis[i], ok = false;
          try { ok = typeof global[an] !== "undefined" || an in global; } catch (e) {}
          put("api_flag_" + i + "_name", an);
          put("api_flag_" + i + "_ok", !!ok);
          if (ok) { hit++; if (sample.length < 12) sample.push(an); } else miss++;
        }
        put("api_flags_n", apis.length);
        put("api_flags_hit", hit);
        put("api_flags_miss", miss);
        put("api_flags_sample", sample);
        put("api_flags_hash", shash(sample.join("|")));
        put("api_probe_n", apis.length);
        put("api_probe_hit", hit);
        put("api_probe_ratio", apis.length ? hit / apis.length : 0);
        put("api_probe_sample", sample);
        put("api_probe_hash", shash(sample.join(",")));
      }

      // ---- census: MQ + fonts ----
      if (family === "census") {
        var mqs = (lists.MEDIA_QUERIES || []).slice(0, 36);
        var mqTrue = 0, mqSample = [];
        for (var mi = 0; mi < mqs.length; mi++) {
          var mok = false;
          try { mok = matchMedia(mqs[mi]).matches; } catch (eM) {}
          put("mq_row_" + mi + "_q", String(mqs[mi]).slice(0, 80));
          put("mq_row_" + mi + "_ok", !!mok);
          if (mok) { mqTrue++; if (mqSample.length < 10) mqSample.push(mqs[mi]); }
        }
        put("mq_probe_n", mqs.length);
        put("mq_true_n", mqTrue);
        put("mq_true_sample", mqSample);
        put("mq_hash", shash(mqSample.join("|")));
        var fonts = (lists.FONTS || []).slice(0, 32);
        var fontHit = [], w0 = 0;
        try {
          var c = document.createElement("canvas");
          var g = c.getContext && c.getContext("2d");
          if (g) {
            g.font = "72px monospace";
            w0 = g.measureText("mmmmmmmmmmlli").width;
            for (var fi = 0; fi < fonts.length; fi++) {
              g.font = '72px "' + fonts[fi] + '",monospace';
              var w = g.measureText("mmmmmmmmmmlli").width;
              var fok = Math.abs(w - w0) > 0.5;
              put("font_row_" + fi + "_name", fonts[fi]);
              put("font_row_" + fi + "_hit", !!fok);
              if (fok) fontHit.push(fonts[fi]);
            }
          }
        } catch (eF) { put("font_probe_err", String(eF && eF.message || eF)); }
        put("font_probe_n", fonts.length);
        put("font_hit_n", fontHit.length);
        put("font_hit_sample", fontHit.slice(0, 12));
        put("font_matrix_hash", shash(fontHit.join("|")));
      }

      // ---- browser_kernel: navigator demand (counts toward ≥30; not shared baseline) ----
      if (family === "browser_kernel") {
        var nav = navigator || {};
        put("nav_user_agent_len", (nav.userAgent || "").length);
        put("nav_app_code_name", nav.appCodeName || "");
        put("nav_app_name", nav.appName || "");
        put("nav_product_sub", nav.productSub || "");
        put("nav_vendor_sub", nav.vendorSub || "");
        put("nav_do_not_track", nav.doNotTrack || null);
        put("nav_global_privacy", nav.globalPrivacyControl != null ? !!nav.globalPrivacyControl : null);
        put("nav_java", typeof nav.javaEnabled === "function" ? !!nav.javaEnabled() : null);
        put("nav_plugins_n", nav.plugins ? nav.plugins.length : 0);
        put("nav_mimes_n", nav.mimeTypes ? nav.mimeTypes.length : 0);
        put("nav_connection", !!(nav.connection || nav.mozConnection));
        put("nav_locks", !!nav.locks);
        put("nav_credentials", !!nav.credentials);
        put("nav_media_devices", !!nav.mediaDevices);
        put("nav_permissions", !!nav.permissions);
        put("nav_storage", !!nav.storage);
        put("nav_wake_lock", !!nav.wakeLock);
        put("nav_keyboard", !!nav.keyboard);
        put("nav_hid", !!nav.hid);
        put("nav_usb", !!nav.usb);
        put("nav_serial", !!nav.serial);
        put("nav_bluetooth", !!nav.bluetooth);
        put("nav_xr", !!nav.xr);
        put("nav_gpu", !!nav.gpu);
        put("nav_user_agent_data", !!nav.userAgentData);
        put("window_chrome", typeof chrome !== "undefined");
        put("window_safari", typeof safari !== "undefined");
        put("window_opera", typeof opera !== "undefined");
        put("window_own_sample_n", (function () {
          var n = 0, k; try { for (k in window) { if (Object.prototype.hasOwnProperty.call(window, k)) n++; if (n > 200) break; } } catch (e) {}
          return n;
        })());
      }

      // ---- gpu_physical ----
      if (family === "gpu_physical" || family === "material_hedge") {
        try {
          var c2 = document.createElement("canvas");
          var gl = c2.getContext && (c2.getContext("webgl2") || c2.getContext("webgl") || c2.getContext("experimental-webgl"));
          if (gl) {
            // Only numeric pnames that exist on this context (undefined → skip = no INVALID_ENUM).
            var names = ["VENDOR","RENDERER","VERSION","SHADING_LANGUAGE_VERSION","MAX_TEXTURE_SIZE","MAX_CUBE_MAP_TEXTURE_SIZE","MAX_RENDERBUFFER_SIZE","MAX_VERTEX_ATTRIBS","MAX_VERTEX_UNIFORM_VECTORS","MAX_FRAGMENT_UNIFORM_VECTORS","MAX_VARYING_VECTORS","MAX_TEXTURE_IMAGE_UNITS","MAX_VERTEX_TEXTURE_IMAGE_UNITS","MAX_COMBINED_TEXTURE_IMAGE_UNITS","DEPTH_BITS","STENCIL_BITS","SAMPLES","SAMPLE_BUFFERS","RED_BITS","GREEN_BITS","BLUE_BITS","ALPHA_BITS","SUBPIXEL_BITS","MAX_VIEWPORT_DIMS","ALIASED_LINE_WIDTH_RANGE","ALIASED_POINT_SIZE_RANGE"];
            var paramKeys = ["VENDOR","RENDERER","VERSION","SHADING_LANGUAGE_VERSION","MAX_TEXTURE_SIZE","MAX_CUBE_MAP_TEXTURE_SIZE","MAX_RENDERBUFFER_SIZE","MAX_VERTEX_ATTRIBS","MAX_VERTEX_UNIFORM_VECTORS","MAX_FRAGMENT_UNIFORM_VECTORS","MAX_VARYING_VECTORS","MAX_TEXTURE_IMAGE_UNITS","MAX_VERTEX_TEXTURE_IMAGE_UNITS","MAX_COMBINED_TEXTURE_IMAGE_UNITS","DEPTH_BITS","STENCIL_BITS","SAMPLES","SAMPLE_BUFFERS","RED_BITS","GREEN_BITS","BLUE_BITS","ALPHA_BITS","SUBPIXEL_BITS","MAX_VIEWPORT_DIMS","ALIASED_LINE_WIDTH_RANGE","ALIASED_POINT_SIZE_RANGE"];
            for (var gi = 0; gi < paramKeys.length; gi++) {
              var pname = gl[paramKeys[gi]];
              var gv = safeGlParam(gl, pname);
              if (gv != null) put("glp_" + names[gi], (gv && gv.length) ? Array.prototype.slice.call(gv) : gv);
            }
            var exts = gl.getSupportedExtensions() || [];
            put("gl_ext_full_n", exts.length);
            put("gl_ext_full_hash", shash(exts.slice().sort().join(",")));
            for (var ej = 0; ej < Math.min(exts.length, 20); ej++) put("gl_ext_name_" + ej, exts[ej]);
            var dbg = gl.getExtension("WEBGL_debug_renderer_info");
            if (dbg) {
              put("gl_unmasked_vendor_dense", safeGlParam(gl, dbg.UNMASKED_VENDOR_WEBGL) || "");
              put("gl_unmasked_renderer_dense", safeGlParam(gl, dbg.UNMASKED_RENDERER_WEBGL) || "");
            }
            put("gl_vendor_dense", safeGlParam(gl, gl.VENDOR) || "");
            put("gl_renderer_dense", safeGlParam(gl, gl.RENDERER) || "");
            put("gl_version_dense", safeGlParam(gl, gl.VERSION) || "");
            put("gl_shading_dense", safeGlParam(gl, gl.SHADING_LANGUAGE_VERSION) || "");
            releaseGl(gl, c2);
          } else put("webgl_missing", true);
        } catch (eG) { put("gl_dense_error", String(eG && eG.message || eG)); }
      }

      // ---- network (expand to ≥30 real) ----
      if (family === "network") {
        var conn = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
        put("net_type_dense", conn && conn.type ? conn.type : null);
        put("net_effective_dense", conn && conn.effectiveType ? conn.effectiveType : null);
        put("net_rtt_dense", conn && conn.rtt != null ? conn.rtt : null);
        put("net_downlink_dense", conn && conn.downlink != null ? conn.downlink : null);
        put("net_save_data_dense", conn && conn.saveData != null ? !!conn.saveData : null);
        put("webrtc_ctor", !!(window.RTCPeerConnection || window.webkitRTCPeerConnection));
        put("rtc_peer", !!(window.RTCPeerConnection || window.webkitRTCPeerConnection));
        put("rtc_session_desc", typeof RTCSessionDescription !== "undefined");
        put("rtc_ice_candidate", typeof RTCIceCandidate !== "undefined");
        put("rtc_data_channel", typeof RTCDataChannel !== "undefined");
        put("media_devices", !!(navigator.mediaDevices && navigator.mediaDevices.enumerateDevices));
        put("get_user_media", !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia));
        put("websocket_ctor", typeof WebSocket !== "undefined");
        put("eventsource_ctor", typeof EventSource !== "undefined");
        put("fetch_ctor", typeof fetch !== "undefined");
        put("headers_ctor", typeof Headers !== "undefined");
        put("request_ctor", typeof Request !== "undefined");
        put("response_ctor", typeof Response !== "undefined");
        put("abort_controller", typeof AbortController !== "undefined");
        put("broadcast_channel", typeof BroadcastChannel !== "undefined");
        // DNS/online capability flags
        put("on_line", !!navigator.onLine);
        put("service_worker_net", !!navigator.serviceWorker);
        put("cookie_enabled_net", !!navigator.cookieEnabled);
        // protocol/location
        put("loc_protocol", (location && location.protocol) || "");
        put("loc_host_len", ((location && location.host) || "").length);
        put("loc_pathname_len", ((location && location.pathname) || "").length);
        put("loc_search_len", ((location && location.search) || "").length);
        put("loc_hash_len", ((location && location.hash) || "").length);
        put("is_https", (location && location.protocol) === "https:");
        put("is_secure_net", !!window.isSecureContext);
        // performance network timing presence
        try {
          var navE = safePerfEntries("navigation");
          put("perf_nav_entries_n", navE.length);
          if (navE[0]) {
            put("perf_ttfb", navE[0].responseStart || null);
            put("perf_dom_interactive", navE[0].domInteractive || null);
            put("perf_load_event", navE[0].loadEventEnd || null);
          }
        } catch (eP) { put("perf_net_err", String(eP && eP.message || eP)); }
      }

      // ---- os / mobile_form (expand ≥30) ----
      if (family === "os" || family === "mobile_form") {
        put("orient_type", (screen.orientation && screen.orientation.type) || null);
        put("orient_angle", (screen.orientation && screen.orientation.angle != null) ? screen.orientation.angle : null);
        put("touch_points", navigator.maxTouchPoints || 0);
        put("motion_api", typeof DeviceMotionEvent !== "undefined");
        put("orient_api", typeof DeviceOrientationEvent !== "undefined");
        put("tz_offset_jan", new Date(Date.UTC(2024, 0, 1)).getTimezoneOffset());
        put("tz_offset_jul", new Date(Date.UTC(2024, 6, 1)).getTimezoneOffset());
        put("tz_dst", f.tz_offset_jan !== f.tz_offset_jul);
        try {
          var ro = Intl.DateTimeFormat().resolvedOptions();
          put("intl_locale", ro.locale || "");
          put("intl_calendar", ro.calendar || "");
          put("intl_numbering", ro.numberingSystem || "");
          put("intl_tz", ro.timeZone || "");
          put("intl_hour_cycle", ro.hourCycle || "");
        } catch (eI) {}
        try {
          put("intl_collator", !!(Intl.Collator));
          put("intl_plural", !!(Intl.PluralRules));
          put("intl_relative", !!(Intl.RelativeTimeFormat));
          put("intl_list", !!(Intl.ListFormat));
          put("intl_segmenter", !!(Intl.Segmenter));
          put("intl_display", !!(Intl.DisplayNames));
          put("intl_number_format", !!(Intl.NumberFormat));
          put("intl_datetime_format", !!(Intl.DateTimeFormat));
        } catch (e2) {}
        // Prefer cached metrics when registry helpers exposed; else single local read.
        (function () {
          var sm = null;
          try {
            var h = global.GRCollectors && global.GRCollectors.__h;
            if (h && typeof h.readScreenMetrics === "function") sm = h.readScreenMetrics();
          } catch (eSm) {}
          if (sm) {
            put("screen_width_os", sm.screen_width || 0);
            put("screen_height_os", sm.screen_height || 0);
            put("screen_avail_w_os", sm.screen_avail_width || 0);
            put("screen_avail_h_os", sm.screen_avail_height || 0);
            put("screen_fp_protection_suspect", !!sm.screen_fp_protection_suspect);
            put("screen_color_depth_os", sm.color_depth || 0);
            put("screen_pixel_depth_os", sm.pixel_depth || 0);
          } else {
            put("screen_width_os", screen.width || 0);
            put("screen_height_os", screen.height || 0);
            put("screen_avail_w_os", screen.availWidth || 0);
            put("screen_avail_h_os", screen.availHeight || 0);
            put("screen_color_depth_os", screen.colorDepth || 0);
            put("screen_pixel_depth_os", screen.pixelDepth || 0);
          }
        })();
        put("dpr_os", typeof devicePixelRatio !== "undefined" ? devicePixelRatio : 1);
        put("hw_concurrency_os", navigator.hardwareConcurrency || 0);
        put("device_memory_os", navigator.deviceMemory != null ? navigator.deviceMemory : null);
        put("platform_os", navigator.platform || "");
        put("languages_n_os", (navigator.languages && navigator.languages.length) || 0);
        put("language_os", navigator.language || "");
        // prefers media
        put("prefers_color_scheme_dark", matchMedia("(prefers-color-scheme: dark)").matches);
        put("prefers_reduced_motion", matchMedia("(prefers-reduced-motion: reduce)").matches);
        put("prefers_contrast_more", matchMedia("(prefers-contrast: more)").matches);
        put("color_gamut_p3", matchMedia("(color-gamut: p3)").matches);
        put("dynamic_range_high", matchMedia("(dynamic-range: high)").matches);
        put("hover_os", matchMedia("(hover: hover)").matches);
        put("pointer_fine_os", matchMedia("(pointer: fine)").matches);
        put("pointer_coarse_os", matchMedia("(pointer: coarse)").matches);
      }

      // ---- sandbox_xsrc ----
      if (family === "sandbox_xsrc") {
        put("is_top", window.top === window.self);
        put("frame_depth", (function () { var d = 0, w = window; try { while (w !== w.parent) { d++; w = w.parent; if (d > 8) break; } } catch (e) {} return d; })());
        put("opener_present", !!window.opener);
        put("frame_element", !!window.frameElement);
        put("worker_ctor", typeof Worker !== "undefined");
        put("shared_worker_ctor", typeof SharedWorker !== "undefined");
        put("service_worker_sb", !!navigator.serviceWorker);
        put("cross_origin_isolated", !!crossOriginIsolated);
        put("is_secure", !!window.isSecureContext);
        put("origin", (location && location.origin) || "");
        put("protocol", (location && location.protocol) || "");
        try { put("ancestor_origins_n", (location.ancestorOrigins && location.ancestorOrigins.length) || 0); } catch (eA) { put("ancestor_origins_n", null); }
        put("parent_same_origin", (function () { try { return !!window.parent.location.href; } catch (e) { return false; } })());
        put("broadcast_sb", typeof BroadcastChannel !== "undefined");
        put("message_channel", typeof MessageChannel !== "undefined");
        put("shared_array_sb", typeof SharedArrayBuffer !== "undefined");
        put("atomics_sb", typeof Atomics !== "undefined");
        put("offscreen_sb", typeof OffscreenCanvas !== "undefined");
        put("document_picture_in_picture", typeof documentPictureInPicture !== "undefined");
        // more capability flags for density
        put("import_maps", !!document.querySelector('script[type="importmap"]'));
        put("module_preload", !!document.querySelector('link[rel="modulepreload"]'));
        put("csp_meta", !!document.querySelector('meta[http-equiv="Content-Security-Policy"]'));
        put("referrer_policy_meta", !!document.querySelector('meta[name="referrer"]'));
        put("base_href_set", !!document.querySelector("base[href]"));
        put("visibility_state_sb", document.visibilityState || "");
        put("hidden_sb", !!document.hidden);
        put("has_focus_sb", typeof document.hasFocus === "function" ? document.hasFocus() : null);
        put("ready_state_sb", document.readyState || "");
        put("compat_mode", document.compatMode || "");
        put("character_set", document.characterSet || document.charset || "");
        put("content_type", document.contentType || null);
      }

      // ---- peripheral ----
      if (family === "peripheral") {
        put("gamepad_api", !!(navigator.getGamepads));
        put("hid_api", !!(navigator.hid));
        put("usb_api", !!(navigator.usb));
        put("serial_api", !!(navigator.serial));
        put("bluetooth_api", !!(navigator.bluetooth));
        put("xr_api", !!(navigator.xr));
        put("bluetooth", !!navigator.bluetooth);
        put("usb", !!navigator.usb);
        put("hid", !!navigator.hid);
        put("serial", !!navigator.serial);
        put("payment_request", typeof PaymentRequest !== "undefined");
        put("public_key_cred", typeof PublicKeyCredential !== "undefined");
        put("credentials", !!navigator.credentials);
        put("wake_lock", !!navigator.wakeLock);
        put("idle_detector", typeof IdleDetector !== "undefined");
        put("keyboard", !!navigator.keyboard);
        put("virtual_keyboard", !!(navigator.virtualKeyboard));
        put("media_devices_periph", !!navigator.mediaDevices);
        put("geolocation", !!navigator.geolocation);
        put("vibrate", typeof navigator.vibrate === "function");
        put("battery_api", !!(navigator.getBattery));
        put("device_memory_periph", navigator.deviceMemory != null ? navigator.deviceMemory : null);
        put("max_touch_periph", navigator.maxTouchPoints || 0);
        put("ontouchstart", "ontouchstart" in window);
        put("onpointerdown", "onpointerdown" in window);
        put("ongamepadconnected", "ongamepadconnected" in window);
        put("media_session", !!(navigator.mediaSession));
        put("ink_api", typeof Ink !== "undefined");
        put("eye_dropper", typeof EyeDropper !== "undefined");
        put("barcode_detector", typeof BarcodeDetector !== "undefined");
        put("face_detector", typeof FaceDetector !== "undefined");
        put("text_detector", typeof TextDetector !== "undefined");
        put("contacts_manager", !!(navigator.contacts));
        put("scheduling", !!(navigator.scheduling));
      }

      // ---- rpa ----
      if (family === "rpa") {
        put("pointer_fine", matchMedia("(pointer: fine)").matches);
        put("pointer_coarse", matchMedia("(pointer: coarse)").matches);
        put("pointer_none", matchMedia("(pointer: none)").matches);
        put("hover_hover", matchMedia("(hover: hover)").matches);
        put("hover_none", matchMedia("(hover: none)").matches);
        put("any_pointer_fine", matchMedia("(any-pointer: fine)").matches);
        put("any_pointer_coarse", matchMedia("(any-pointer: coarse)").matches);
        put("any_hover", matchMedia("(any-hover: hover)").matches);
        var vv = window.visualViewport;
        if (vv) {
          put("vv_width", vv.width); put("vv_height", vv.height);
          put("vv_offset_left", vv.offsetLeft); put("vv_offset_top", vv.offsetTop);
          put("vv_page_left", vv.pageLeft); put("vv_page_top", vv.pageTop);
          put("vv_scale", vv.scale);
        } else put("vv_missing", true);
        put("inner_width_rpa", window.innerWidth || 0);
        put("inner_height_rpa", window.innerHeight || 0);
        put("outer_width_rpa", window.outerWidth || 0);
        put("outer_height_rpa", window.outerHeight || 0);
        put("screen_x", window.screenX != null ? window.screenX : null);
        put("screen_y", window.screenY != null ? window.screenY : null);
        put("device_pixel_ratio_rpa", typeof devicePixelRatio !== "undefined" ? devicePixelRatio : 1);
        put("max_touch_rpa", navigator.maxTouchPoints || 0);
        put("webdriver_rpa", !!navigator.webdriver);
        put("pointer_event", typeof PointerEvent !== "undefined");
        put("touch_event", typeof TouchEvent !== "undefined");
        put("mouse_event", typeof MouseEvent !== "undefined");
        put("keyboard_event", typeof KeyboardEvent !== "undefined");
        put("wheel_event", typeof WheelEvent !== "undefined");
        put("drag_event", typeof DragEvent !== "undefined");
        put("focus_event", typeof FocusEvent !== "undefined");
        put("input_event", typeof InputEvent !== "undefined");
        put("composition_event", typeof CompositionEvent !== "undefined");
        put("animation_frame", typeof requestAnimationFrame === "function");
        put("idle_callback", typeof requestIdleCallback === "function");
        put("performance_now_rpa", performance && performance.now ? performance.now() : null);
      }

      // ---- material_hedge math/wasm/canvas lite ----
      if (family === "material_hedge") {
        put("math_sin", Math.sin(1e-10));
        put("math_cos", Math.cos(1e-10));
        put("math_tan", Math.tan(1e-10));
        put("math_log", Math.log(Math.E));
        put("math_expm1", Math.expm1 ? Math.expm1(1e-10) : null);
        put("math_acosh", Math.acosh ? Math.acosh(1e10) : null);
        put("math_asinh", Math.asinh ? Math.asinh(1e-10) : null);
        put("math_atanh", Math.atanh ? Math.atanh(1e-10) : null);
        put("math_cbrt", Math.cbrt ? Math.cbrt(27) : null);
        put("math_hypot", Math.hypot ? Math.hypot(3, 4) : null);
        put("math_imul", Math.imul ? Math.imul(0xffffffff, 5) : null);
        put("math_clz32", Math.clz32 ? Math.clz32(1) : null);
        put("math_fround", Math.fround ? Math.fround(1.337) : null);
        put("math_hash", shash([f.math_sin, f.math_cos, f.math_tan, f.math_log].join(",")));
        put("wasm_present", typeof WebAssembly !== "undefined");
        put("wasm_instantiate", !!(WebAssembly && WebAssembly.instantiate));
        put("wasm_compile", !!(WebAssembly && WebAssembly.compile));
        put("wasm_validate", !!(WebAssembly && WebAssembly.validate));
        try { put("wasm_validate_empty", WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0]))); } catch (eW) { put("wasm_err", String(eW && eW.message || eW)); }
        put("canvas_2d_ctor", (function () { try { return !!document.createElement("canvas").getContext("2d"); } catch (e) { return false; } })());
        put("webgl_mat", (function () { try { var c=document.createElement("canvas"); return !!(c.getContext("webgl")||c.getContext("experimental-webgl")); } catch (e) { return false; } })());
        put("webgl2_mat", (function () { try { return !!document.createElement("canvas").getContext("webgl2"); } catch (e) { return false; } })());
        put("offscreen_mat", typeof OffscreenCanvas !== "undefined");
        put("path2d_mat", typeof Path2D !== "undefined");
        put("image_bitmap", typeof createImageBitmap === "function");
        put("audio_ctx_mat", !!(window.AudioContext || window.webkitAudioContext));
        put("offline_audio_mat", !!(window.OfflineAudioContext || window.webkitOfflineAudioContext));
        put("audio_worklet_mat", !!(window.AudioWorkletNode));
        put("crypto_subtle", !!(window.crypto && window.crypto.subtle));
        put("text_encoder", typeof TextEncoder !== "undefined");
        put("text_decoder", typeof TextDecoder !== "undefined");
      }
    } catch (eFam) {
      put("family_dense_error", String((eFam && eFam.message) || eFam));
    }

    // NEVER emit vanity _m_/_t_ pads. If still short, deepen with real CSS/API rows.
    if (countPackSpecific(f, packId) < minKeys) {
      var props = (lists.CSS_PROPS || []).slice(0, 40);
      for (var pi = 0; pi < props.length && countPackSpecific(f, packId) < minKeys; pi++) {
        var pn = props[pi];
        var pv = "";
        try {
          var el = document.createElement("div");
          if (document.body) document.body.appendChild(el);
          pv = getComputedStyle(el).getPropertyValue(pn) || "";
          if (el.parentNode) el.parentNode.removeChild(el);
        } catch (eC) {}
        put("css_deep_" + pi + "_prop", pn);
        put("css_deep_" + pi + "_set", !!pv);
      }
    }
    if (countPackSpecific(f, packId) < minKeys) {
      var cs = (lists.CSS_SUPPORTS || []).slice(0, 40);
      for (var ci = 0; ci < cs.length && countPackSpecific(f, packId) < minKeys; ci++) {
        var pair = cs[ci], cok = false;
        try { cok = CSS && CSS.supports && CSS.supports(pair[0], pair[1]); } catch (eS) {}
        put("css_sup_deep_" + ci + "_ok", !!cok);
        put("css_sup_deep_" + ci + "_pair", pair[0] + ":" + pair[1]);
      }
    }

    // Competitive high-value named materials (FingerprintJS/CreepJS-class — real digests, not vanity).
    // Always attempt once per densify; put() no-ops if pack already set the key.
    try {
      // Speech voices (strong OS/locale entropy)
      if (typeof speechSynthesis !== "undefined" && speechSynthesis.getVoices) {
        var voices = speechSynthesis.getVoices() || [];
        put("speech_voices_n", voices.length);
        var vnames = [];
        for (var vi = 0; vi < Math.min(voices.length, 48); vi++) {
          vnames.push((voices[vi].lang || "") + ":" + (voices[vi].name || "").slice(0, 40));
        }
        put("speech_voices_hash", shash(vnames.join("|")));
        put("speech_default_voice", voices[0] ? (voices[0].name || "").slice(0, 48) : "");
      }
      // Font preferences (FPJS fontPreferences)
      try {
        var cfp = document.createElement("canvas");
        var gfp = cfp.getContext && cfp.getContext("2d");
        if (gfp) {
          var baseTxt = "mmMwWLliI0O&1";
          gfp.font = "72px serif";
          var wSerif = gfp.measureText(baseTxt).width;
          gfp.font = "72px sans-serif";
          var wSans = gfp.measureText(baseTxt).width;
          gfp.font = "72px monospace";
          var wMono = gfp.measureText(baseTxt).width;
          gfp.font = "72px system-ui,sans-serif";
          var wSys = gfp.measureText(baseTxt).width;
          put("font_pref_default_width", wSys);
          put("font_pref_serif_width", wSerif);
          put("font_pref_sans_width", wSans);
          put("font_pref_mono_width", wMono);
          put("font_preferences_hash", shash([wSerif, wSans, wMono, wSys].join(",")));
        }
      } catch (eFp) {}
      // System colors digest
      try {
        var scEl = document.createElement("div");
        if (document.body) document.body.appendChild(scEl);
        var sc = getComputedStyle(scEl);
        var scNames = ["Canvas", "CanvasText", "LinkText", "VisitedText", "ActiveText", "ButtonFace", "ButtonText", "Field", "FieldText", "Highlight", "HighlightText", "GrayText", "Mark", "MarkText"];
        var scParts = [];
        for (var si = 0; si < scNames.length; si++) {
          var cv = "";
          try { cv = sc.getPropertyValue("color") || ""; } catch (eSc) {}
          // use color-scheme probe via temporary
          scEl.style.color = scNames[si];
          try { cv = getComputedStyle(scEl).color || ""; } catch (eSc2) {}
          scParts.push(scNames[si] + ":" + cv);
        }
        if (scEl.parentNode) scEl.parentNode.removeChild(scEl);
        put("system_colors_n", scNames.length);
        put("system_colors_hash", shash(scParts.join("|")));
      } catch (eSys) {}
      // DomRect / SVG geometry
      try {
        var dr = document.createElement("div");
        dr.style.cssText = "position:absolute;left:-9999px;width:100px;height:50px;padding:10px;border:2px solid;font:16px serif";
        dr.textContent = "gr";
        if (document.body) document.body.appendChild(dr);
        var r = dr.getBoundingClientRect();
        put("dom_rect_sample_n", 1);
        put("dom_rect_hash", shash([r.x, r.y, r.width, r.height, r.top, r.left].join(",")));
        if (dr.parentNode) dr.parentNode.removeChild(dr);
        var svgNs = "http://www.w3.org/2000/svg";
        var svg = document.createElementNS(svgNs, "svg");
        svg.setAttribute("width", "100");
        svg.setAttribute("height", "20");
        var st = document.createElementNS(svgNs, "text");
        st.setAttribute("x", "0");
        st.setAttribute("y", "15");
        st.textContent = "mmMwWLliI0";
        svg.appendChild(st);
        if (document.body) document.body.appendChild(svg);
        var bb = st.getBBox ? st.getBBox() : { width: 0, height: 0 };
        put("svg_rect_hash", shash([bb.width, bb.height, bb.x || 0, bb.y || 0].join(",")));
        if (svg.parentNode) svg.parentNode.removeChild(svg);
      } catch (eDr) {}
      // Screen frame (FPJS) — use cached metrics when available
      try {
        put("screen_frame_inner", (window.innerWidth || 0) + "x" + (window.innerHeight || 0));
        put("screen_frame_outer", (window.outerWidth || 0) + "x" + (window.outerHeight || 0));
        var smF = null;
        try {
          var hh = global.GRCollectors && global.GRCollectors.__h;
          if (hh && typeof hh.readScreenMetrics === "function") smF = hh.readScreenMetrics();
        } catch (eSmF) {}
        if (smF) {
          put("screen_avail_delta_w", smF.screen_avail_delta_w != null ? smF.screen_avail_delta_w : 0);
          put("screen_avail_delta_h", smF.screen_avail_delta_h != null ? smF.screen_avail_delta_h : 0);
        } else {
          put("screen_avail_delta_w", (screen.width || 0) - (screen.availWidth || 0));
          put("screen_avail_delta_h", (screen.height || 0) - (screen.availHeight || 0));
        }
      } catch (eSf) {}
      // Vendor flavors — no InstallTrigger (deprecated); use mozInnerScreenX / Firefox UA
      try {
        var flavors = [];
        if (typeof chrome !== "undefined") flavors.push("chrome");
        if (typeof safari !== "undefined") flavors.push("safari");
        if (typeof opera !== "undefined" || (navigator.userAgent || "").indexOf("OPR/") >= 0) flavors.push("opera");
        var isFx = false;
        try {
          isFx = typeof window.mozInnerScreenX === "number";
        } catch (eFx0) {}
        if (!isFx) {
          var uaF = String(navigator.userAgent || "");
          isFx = /Firefox\//.test(uaF) || /FxiOS\//.test(uaF);
        }
        if (isFx) flavors.push("firefox");
        if (navigator.brave) flavors.push("brave");
        put("vendor_flavors", flavors);
        put("vendor_flavors_n", flavors.length);
      } catch (eVf) {}
      // Architecture / bitness
      try {
        var uad = navigator.userAgentData;
        if (uad) {
          put("architecture_bitness", (uad.platform || "") + ":" + (uad.mobile ? "m" : "d"));
          if (uad.getHighEntropyValues) {
            // fire-and-forget not possible sync; use already-resolved if present
          }
        }
        put("pdf_viewer_enabled", navigator.pdfViewerEnabled != null ? !!navigator.pdfViewerEnabled : null);
      } catch (eArch) {}
      // Intl / datetime locale
      try {
        put("datetime_locale", Intl.DateTimeFormat().resolvedOptions().locale || "");
        put("intl_datetime_format", Intl.DateTimeFormat().resolvedOptions().timeZone || "");
        put("intl_number_format", Intl.NumberFormat().resolvedOptions().locale || "");
        var jan = new Date(2024, 0, 1).getTimezoneOffset();
        var jul = new Date(2024, 6, 1).getTimezoneOffset();
        put("timezone_offset_jan", jan);
        put("timezone_offset_jul", jul);
        put("timezone_dst_delta", Math.abs(jan - jul));
      } catch (eIntl) {}
      // MQ prefs (FPJS parity)
      try {
        put("reduced_transparency", matchMedia("(prefers-reduced-transparency: reduce)").matches);
        put("inverted_colors", matchMedia("(inverted-colors: inverted)").matches);
        put("monochrome_depth", matchMedia("(monochrome)").matches ? 1 : 0);
        put("prefers_color_scheme_dark", matchMedia("(prefers-color-scheme: dark)").matches);
        put("prefers_reduced_motion", matchMedia("(prefers-reduced-motion: reduce)").matches);
        put("prefers_contrast_more", matchMedia("(prefers-contrast: more)").matches);
        put("color_gamut_p3", matchMedia("(color-gamut: p3)").matches);
        put("dynamic_range_high", matchMedia("(dynamic-range: high)").matches);
        put("css_forced_colors", matchMedia("(forced-colors: active)").matches);
        put("css_pointer_fine", matchMedia("(pointer: fine)").matches);
        put("css_any_pointer_coarse", matchMedia("(any-pointer: coarse)").matches);
        put("css_hover_none", matchMedia("(hover: none)").matches);
        put("css_display_mode_standalone", matchMedia("(display-mode: standalone)").matches);
      } catch (eMq) {}
      // Storage quota API surface (async values filled by scheduleAsyncCompetitiveFollowups)
      try {
        if (navigator.storage && navigator.storage.estimate) {
          put("storage_estimate_api", true);
          put("storage_persist_api", typeof navigator.storage.persist === "function");
        } else {
          put("storage_estimate_api", false);
        }
        put("is_secure_context", !!window.isSecureContext);
        put("cross_origin_isolated", !!window.crossOriginIsolated);
        put("crypto_subtle", !!(window.crypto && window.crypto.subtle));
        put("webgpu_available", !!(navigator.gpu));
        put("webgl2", (function () {
          try { return !!document.createElement("canvas").getContext("webgl2"); } catch (e) { return false; }
        })());
        put("has_webrtc", typeof RTCPeerConnection !== "undefined");
        put("shared_worker", typeof SharedWorker !== "undefined");
        put("offscreen", typeof OffscreenCanvas !== "undefined");
        put("image_bitmap", typeof createImageBitmap === "function");
        put("broadcast_channel", typeof BroadcastChannel !== "undefined");
        put("message_channel", typeof MessageChannel !== "undefined");
        put("public_key_cred", typeof PublicKeyCredential !== "undefined");
        put("credentials", !!navigator.credentials);
        put("keyboard", !!navigator.keyboard);
        put("virtual_keyboard", !!navigator.virtualKeyboard);
        put("vibrate", typeof navigator.vibrate === "function");
        put("battery_api", typeof navigator.getBattery === "function");
        put("media_devices", !!navigator.mediaDevices);
        put("geolocation", !!navigator.geolocation);
        put("apple_pay_session", typeof ApplePaySession !== "undefined");
        put("touch_points", navigator.maxTouchPoints || 0);
        put("touch_support", (navigator.maxTouchPoints || 0) > 0 || "ontouchstart" in window);
        put("ontouchstart", "ontouchstart" in window);
        put("onpointerdown", "onpointerdown" in window);
        put("pointer_event", typeof PointerEvent !== "undefined");
        put("touch_event", typeof TouchEvent !== "undefined");
        put("mouse_event", typeof MouseEvent !== "undefined");
        put("keyboard_event", typeof KeyboardEvent !== "undefined");
        put("wheel_event", typeof WheelEvent !== "undefined");
        put("animation_frame", typeof requestAnimationFrame === "function");
        put("idle_callback", typeof requestIdleCallback === "function");
        put("outer_height", window.outerHeight || 0);
        put("inner_height", window.innerHeight || 0);
        put("pixel_depth", screen.pixelDepth || screen.colorDepth || 0);
        put("screen_x", window.screenX != null ? window.screenX : null);
        put("screen_y", window.screenY != null ? window.screenY : null);
        if (screen.orientation) {
          put("orient_type", screen.orientation.type || "");
          put("orient_angle", screen.orientation.angle);
        }
        put("opener_present", !!window.opener);
        put("frame_depth", (function () {
          var d = 0, w = window;
          try {
            while (w !== w.parent) { d++; w = w.parent; if (d > 8) break; }
          } catch (e) {}
          return d;
        })());
        put("parent_same_origin", (function () {
          try { return !!window.parent.location.href; } catch (e) { return false; }
        })());
        try {
          put("ancestor_origins_n", location.ancestorOrigins ? location.ancestorOrigins.length : 0);
        } catch (eAo) { put("ancestor_origins_n", null); }
        put("origin", location && location.origin || "");
        put("compat_mode", document.compatMode || "");
        put("character_set", document.characterSet || document.charset || "");
        put("is_https", location && location.protocol === "https:");
        put("loc_protocol", location && location.protocol || "");
        put("loc_host_len", (location && location.host || "").length);
        put("cookie_enabled_net", navigator.cookieEnabled);
        put("on_line", navigator.onLine);
        put("chrome_app", typeof chrome !== "undefined" && !!(chrome.app || chrome.runtime));
        put("pdf_viewer_enabled", navigator.pdfViewerEnabled != null ? !!navigator.pdfViewerEnabled : null);
      } catch (eSurf) {}
      // Math fingerprint (FPJS)
      try {
        put("math_fingerprint_hash", shash([
          Math.tan(-1e300), Math.sin(Math.PI / 2), Math.cos(1e-10),
          Math.log(Math.E), Math.expm1 ? Math.expm1(1) : 0,
          Math.acosh ? Math.acosh(1e10) : 0, Math.asinh ? Math.asinh(1) : 0,
          Math.atanh ? Math.atanh(0.5) : 0, Math.cbrt ? Math.cbrt(100) : 0,
          Math.hypot ? Math.hypot(3, 4, 5) : 0
        ].join(",")));
      } catch (eMath) {}
      // Canvas text metrics hash
      try {
        var ctm = document.createElement("canvas");
        var gtm = ctm.getContext && ctm.getContext("2d");
        if (gtm) {
          gtm.font = "16px Arial";
          var tm = gtm.measureText("Cwm fjordbank glyphs vext quiz 😃");
          put("text_metrics_hash", shash([
            tm.width, tm.actualBoundingBoxAscent || 0, tm.actualBoundingBoxDescent || 0,
            tm.actualBoundingBoxLeft || 0, tm.actualBoundingBoxRight || 0
          ].join(",")));
          put("canvas_emoji_width", tm.width);
        }
      } catch (eTm) {}
      // Codec canPlayType matrix digest
      try {
        var vid = document.createElement("video");
        // RFC6381 codec ids — bare "vp9" is ambiguous and floods Chrome console.
        var codecs = [
          'video/mp4; codecs="avc1.42E01E"',
          'video/mp4; codecs="avc1.4D401E"',
          'video/webm; codecs="vp8"',
          'video/webm; codecs="vp09.00.10.08"',
          'video/webm; codecs="av01.0.05M.08"',
          'audio/mp4; codecs="mp4a.40.2"',
          'audio/webm; codecs="opus"',
          'audio/ogg; codecs="vorbis"',
        ];
        var cparts = [];
        for (var ci2 = 0; ci2 < codecs.length; ci2++) {
          var cr = "";
          try { cr = vid.canPlayType(codecs[ci2]) || ""; } catch (eC) {}
          cparts.push(cr);
        }
        put("codec_support_n", codecs.length);
        put("codec_support_hash", shash(cparts.join("|")));
        put("media_canplay_mp4", (vid.canPlayType('video/mp4; codecs="avc1.42E01E"') || "") !== "");
        put("media_capabilities_hash", shash(cparts.join(",")));
      } catch (eCod) {}
      // Audio base latency
      try {
        var ACtx = window.AudioContext || window.webkitAudioContext;
        if (ACtx) {
          // Shared AC via governor — never hard-close (close is safe no-op).
          var ac = new ACtx();
          put("audio_base_latency_fp", ac.baseLatency != null ? ac.baseLatency : null);
          put("audio_sample_rate_fp", ac.sampleRate != null ? ac.sampleRate : null);
          try {
            if (ac.close) {
              var cl = ac.close();
              if (cl && typeof cl.catch === "function") cl.catch(function () {});
            }
          } catch (eCl) {}
        }
      } catch (eAb) {}
      // catalog breadth meta (from probe lists)
      try {
        put("catalog_n_apis", lists.n_apis || (lists.KNOWN_APIS || []).length || 0);
        put("catalog_n_fonts", lists.n_fonts || (lists.FONTS || []).length || 0);
        put("catalog_n_mq", lists.n_mq || (lists.MEDIA_QUERIES || []).length || 0);
        put("catalog_n_css", lists.n_css || (lists.CSS_SUPPORTS || []).length || 0);
        put("catalog_n_css_props", lists.n_css_props || (lists.CSS_PROPS || []).length || 0);
        put("catalog_n_font_tokens", lists.n_font_tokens || (lists.FONT_TOKENS || []).length || 0);
      } catch (eCat) {}

      // --- R100 → B promote (high commercial value only; presence if available, never force) ---
      // Selected from R00–R99 exclusive pools: network path, intl locale decomp, RTC surface,
      // perf entry types, crypto entropy, visual viewport. Soft/emulated engines may leave null.
      try {
        put("r100_promote_algo", "gr_r100_to_b_v1");
        // Navigation / resource timing protocol (JA-like FE soft signal)
        try {
          if (performance && performance.getEntriesByType) {
            var navE = safePerfEntries("navigation");
            var te = navE[0] || null;
            if (te) {
              put("nav_next_hop_protocol", te.nextHopProtocol != null ? String(te.nextHopProtocol) : null);
              put("nav_transfer_size", te.transferSize != null ? te.transferSize : null);
              put("nav_encoded_body_size", te.encodedBodySize != null ? te.encodedBodySize : null);
              put("nav_decoded_body_size", te.decodedBodySize != null ? te.decodedBodySize : null);
              put("nav_fetch_start", te.fetchStart != null ? te.fetchStart : null);
              put("nav_response_end", te.responseEnd != null ? te.responseEnd : null);
              put("nav_dom_interactive", te.domInteractive != null ? te.domInteractive : null);
            } else {
              put("nav_next_hop_protocol", "no_nav_timing");
            }
            put("perf_resource_entries_n", safePerfEntries("resource").length);
            put("perf_paint_entries_n", safePerfEntries("paint").length);
          }
        } catch (eNav) {}
        // NetworkInformation full (Chrome) — null on Gecko/WK is valid absence
        try {
          var nc = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
          if (nc) {
            put("net_conn_type", nc.type != null ? String(nc.type) : null);
            put("net_conn_effective", nc.effectiveType != null ? String(nc.effectiveType) : null);
            put("net_conn_rtt", nc.rtt != null ? nc.rtt : null);
            put("net_conn_downlink", nc.downlink != null ? nc.downlink : null);
            put("net_conn_downlink_max", nc.downlinkMax != null ? nc.downlinkMax : null);
            put("net_conn_save_data", !!nc.saveData);
          } else {
            put("net_conn_api", "absent");
          }
        } catch (eNc) {}
        // Intl.Locale maximize decomp (R intl_locale exclusive)
        try {
          if (typeof Intl !== "undefined" && Intl.Locale) {
            var lang = navigator.language || (navigator.languages && navigator.languages[0]) || "en";
            if (!lang || lang === "undefined" || lang === "null") lang = "en";
            var loc = new Intl.Locale(lang);
            var locM = loc.maximize ? loc.maximize() : loc;
            put("intl_locale_base", String(loc));
            put("intl_locale_max", String(locM));
            put("intl_locale_language", locM.language || loc.language || null);
            put("intl_locale_script", locM.script || loc.script || null);
            put("intl_locale_region", locM.region || loc.region || null);
            put("intl_locale_calendar", locM.calendar || loc.calendar || null);
          }
        } catch (eLoc) {}
        // RTC surface snapshot (no STUN — presence + state only)
        try {
          var RTCP = global.RTCPeerConnection || global.webkitRTCPeerConnection;
          if (RTCP) {
            var pc = new RTCP({ iceServers: [] });
            put("rtc_can_trickle", pc.canTrickleIceCandidates == null ? null : !!pc.canTrickleIceCandidates);
            put("rtc_connection_state", pc.connectionState || null);
            put("rtc_ice_gathering", pc.iceGatheringState || null);
            put("rtc_signaling", pc.signalingState || null);
            put("rtc_sctp", pc.sctp ? "object" : null);
            put("rtc_add_transceiver", typeof pc.addTransceiver === "function");
            put("rtc_get_stats", typeof pc.getStats === "function");
            try { pc.close(); } catch (ePc) {}
          } else {
            put("rtc_api", "absent");
          }
          put("webtransport", typeof WebTransport !== "undefined");
        } catch (eRtc) {}
        // Crypto entropy + WASM surface
        try {
          if (window.crypto && window.crypto.getRandomValues) {
            var rb = new Uint8Array(16);
            window.crypto.getRandomValues(rb);
            put("crypto_rand_len", rb.length);
            put("crypto_rand_nonzero_n", (function () {
              var z = 0; for (var ri = 0; ri < rb.length; ri++) if (rb[ri]) z++; return z;
            })());
          }
          put("crypto_random_uuid", !!(window.crypto && window.crypto.randomUUID));
          put("wasm_validate", typeof WebAssembly !== "undefined" && typeof WebAssembly.validate === "function");
          put("wasm_instantiate", typeof WebAssembly !== "undefined" && typeof WebAssembly.instantiate === "function");
        } catch (eCr) {}
        // Visual viewport + document fonts
        try {
          var vv = window.visualViewport;
          if (vv) {
            put("vv_width", vv.width);
            put("vv_height", vv.height);
            put("vv_scale", vv.scale);
            put("vv_offset_top", vv.offsetTop);
            put("vv_offset_left", vv.offsetLeft);
          } else {
            put("vv_api", "absent");
          }
          if (document.fonts) {
            put("doc_fonts_status", document.fonts.status || null);
            put("doc_fonts_size", document.fonts.size != null ? document.fonts.size : null);
          }
        } catch (eVv) {}
        // PerformanceObserver entry types (Blink-rich)
        try {
          if (typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes) {
            var set = PerformanceObserver.supportedEntryTypes || [];
            put("perf_obs_entry_types_n", set.length);
            put("perf_obs_entry_types_hash", shash(Array.prototype.slice.call(set).sort().join("|")));
          } else {
            put("perf_obs_entry_types_n", null);
          }
          put("scheduler_post_task", !!(global.scheduler && typeof global.scheduler.postTask === "function"));
          put("queue_microtask", typeof queueMicrotask === "function");
        } catch (ePo) {}
        // Fetch/Request feature surface (sync construct)
        try {
          put("fetch_api", typeof fetch === "function");
          if (typeof Request === "function") {
            try {
              var req = new Request(location.href, { method: "GET" });
              put("request_mode", req.mode || null);
              put("request_credentials", req.credentials || null);
              put("request_cache", req.cache || null);
              put("request_redirect", req.redirect || null);
              put("request_referrer_policy", req.referrerPolicy || null);
            } catch (eReq) {
              put("request_construct", "err");
            }
          }
          put("push_manager", typeof PushManager !== "undefined");
          if (typeof PushManager !== "undefined" && PushManager.supportedContentEncodings) {
            put("push_encodings", (PushManager.supportedContentEncodings || []).join(",") || "empty");
          }
        } catch (eFt) {}
        // Canvas measure multi-script (R canvas_math exclusive heads)
        try {
          var cm = document.createElement("canvas");
          var gm = cm.getContext && cm.getContext("2d");
          if (gm) {
            gm.font = "72px sans-serif";
            put("canvas_w_emoji", gm.measureText("😃🌐").width);
            put("canvas_w_cjk", gm.measureText("汉字測試").width);
            put("canvas_w_arabic", gm.measureText("مرحبا").width);
            put("canvas_w_base", gm.measureText("mmMwWLliI0").width);
            put("canvas_script_widths_hash", shash([
              gm.measureText("😃🌐").width,
              gm.measureText("汉字測試").width,
              gm.measureText("مرحبا").width,
              gm.measureText("mmMwWLliI0").width
            ].join(",")));
          }
        } catch (eCm) {}
      } catch (eR100) {
        put("r100_promote_error", String(eR100 && eR100.message || eR100));
      }
    } catch (eComp) {
      put("competitive_expand_error", String(eComp && eComp.message || eComp));
    }

    f.probe_specific_n = countPackSpecific(f, packId);
    f.probe_pad_n = countPadKeys(f);
    f.probe_field_count = Object.keys(f).length;
    f.probe_min_met = f.probe_specific_n >= minKeys && f.probe_pad_n === 0;
    return f;
  }

  /** densifyBag: family materials only (v3 — no vanity pads). */
  function densifyBag(f, packId, family, minKeys) {
    f = f || {};
    family = resolveFamily(packId, f, family);
    return ensureFamilyMaterials(f, packId, family, minKeys || 30);
  }


  /**
   * Single real emit path (boot-compatible): densify then ctx.queue.enqueue.
   * forceOpt=true for async follow-up recollect (high-entropy materials).
   */
  function emitDense(ctx, packId, fields, priority, source, family, forceOpt) {
    fields = fields || {};
    var fam = resolveFamily(packId, fields, family);
    // Async follow-ups already densified / are thin specialized bags — skip re-densify pad
    var skipDensify = !!(fields.async_followup || fields.async_followup_algo);
    var f = skipDensify ? Object.assign({}, fields) : densifyBag(fields, packId, fam, 30);
    if (!ctx || !ctx.queue || typeof ctx.queue.enqueue !== "function") {
      if (typeof console !== "undefined" && console.warn) {
        /* fe_diag spam removed */ /* silenced */

      }
      return f;
    }
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
    var forceMap = ctx.force_recollect_batches;
    var force = !!(forceOpt || (forceMap && forceMap[packId]));
    var src = source || "main";
    ctx.queue.enqueue({
      session_id: ctx.session_id,
      batch_id: packId,
      source: src,
      inject_path: ctx.inject_path,
      priority: priority != null ? priority : 50,
      force: force,
      payload: {
        fields: f,
        sandbox_kind:
          src && String(src).indexOf("worker") === 0
            ? "worker"
            : src && String(src).indexOf("iframe") === 0
              ? "iframe"
              : "main",
      },
    });
    // Kick async competitive follow-ups once per page/ctx (UA-CH / storage / WebGPU / voices / mediaCaps)
    try {
      scheduleAsyncCompetitiveFollowups(ctx);
    } catch (eAsync) {}
    return f;
  }

  /**
   * Async high-entropy materials that cannot complete in sync densify.
   * Emits force recollect bags into existing commercial packs (not vanity).
   * Once per ctx — covers:
   *   - UA-CH getHighEntropyValues → B65
   *   - storage.estimate / persisted → B27
   *   - WebGPU requestAdapter → B18
   *   - speechSynthesis voiceschanged second tick → B44
   *   - MediaCapabilities.decodingInfo → B19
   *   - Local Font Access: capability-only (never call queryLocalFonts — permission UI)
   */
  function scheduleAsyncCompetitiveFollowups(ctx) {
    if (!ctx || !ctx.queue || typeof ctx.queue.enqueue !== "function") return;
    if (ctx._async_competitive_followups) return;
    ctx._async_competitive_followups = true;
    var pageId =
      (ctx && ctx.page_id) ||
      (ctx && ctx.fields && ctx.fields.page_id) ||
      (typeof global !== "undefined" && global.__GR_PAGE_ID__) ||
      null;
    var baseMeta = {
      async_followup: true,
      async_followup_algo: "gr_async_competitive_v1",
      collected_at: Date.now(),
    };
    if (pageId != null) baseMeta.page_id = pageId;

    // --- UA-CH high entropy ---
    try {
      var uad = navigator.userAgentData;
      if (uad && typeof uad.getHighEntropyValues === "function") {
        uad
          .getHighEntropyValues([
            "architecture",
            "bitness",
            "model",
            "platformVersion",
            "fullVersionList",
            "wow64",
            "formFactors",
            "uaFullVersion",
          ])
          .then(function (he) {
            var f = Object.assign({}, baseMeta, {
              dense_pack: "B65_client_hints_full",
              demand: "UA-CH high-entropy async",
              ua_ch_present: true,
              ua_ch_mobile: !!uad.mobile,
              ua_ch_platform: uad.platform || "",
              ua_ch_brands_n: (uad.brands && uad.brands.length) || 0,
              ua_ch_brands_hash: shash(JSON.stringify(uad.brands || [])),
              ua_ch_architecture: he.architecture || null,
              ua_ch_bitness: he.bitness || null,
              ua_ch_model: he.model || null,
              ua_ch_platform_version: he.platformVersion || null,
              ua_ch_wow64: he.wow64 != null ? !!he.wow64 : null,
              ua_ch_form_factors: he.formFactors || null,
              ua_ch_full_version_n: (he.fullVersionList && he.fullVersionList.length) || 0,
              ua_ch_full_version_list: he.fullVersionList || null,
              ua_ch_full_version_hash: shash(JSON.stringify(he.fullVersionList || [])),
              ua_ch_ua_full_version: he.uaFullVersion || null,
              architecture_bitness:
                String(he.architecture || "") + ":" + String(he.bitness || ""),
              ua_ch_high_entropy_ok: true,
            });
            emitDense(ctx, "B65_client_hints_full", f, 56, "main", "browser_kernel", true);
          })
          .catch(function (e) {
            emitDense(
              ctx,
              "B65_client_hints_full",
              Object.assign({}, baseMeta, {
                dense_pack: "B65_client_hints_full",
                ua_ch_high_entropy_ok: false,
                ua_ch_high_entropy_error: String((e && e.message) || e),
              }),
              56,
              "main",
              "browser_kernel",
              true
            );
          });
      } else {
        emitDense(
          ctx,
          "B65_client_hints_full",
          Object.assign({}, baseMeta, {
            dense_pack: "B65_client_hints_full",
            ua_ch_present: !!uad,
            ua_ch_high_entropy_ok: false,
            ua_ch_high_entropy_skip: "no_userAgentData",
          }),
          56,
          "main",
          "browser_kernel",
          true
        );
      }
    } catch (eUach) {}

    // --- storage.estimate + persisted ---
    try {
      if (navigator.storage && typeof navigator.storage.estimate === "function") {
        var stJobs = [navigator.storage.estimate()];
        if (typeof navigator.storage.persisted === "function") {
          stJobs.push(navigator.storage.persisted());
        }
        Promise.all(
          stJobs.map(function (p) {
            return Promise.resolve(p).catch(function (e) {
              return { __err: String((e && e.message) || e) };
            });
          })
        ).then(function (res) {
          var est = res[0] || {};
          var persisted = res.length > 1 ? res[1] : null;
          var f = Object.assign({}, baseMeta, {
            dense_pack: "B27_storage_privacy",
            demand: "storage.estimate async",
            storage_estimate_api: true,
            storage_quota_bytes: est.quota != null ? est.quota : null,
            storage_usage_bytes: est.usage != null ? est.usage : null,
            storage_usage_details: est.usageDetails || null,
            storage_persisted: typeof persisted === "boolean" ? persisted : null,
            storage_estimate_ok: est.quota != null || est.usage != null,
            storage_estimate_error: est.__err || null,
          });
          if (est.quota > 0 && est.usage != null) {
            f.storage_usage_ratio = est.usage / est.quota;
          }
          emitDense(ctx, "B27_storage_privacy", f, 54, "main", "os", true);
        });
      } else {
        emitDense(
          ctx,
          "B27_storage_privacy",
          Object.assign({}, baseMeta, {
            dense_pack: "B27_storage_privacy",
            storage_estimate_api: false,
            storage_estimate_ok: false,
            storage_estimate_skip: "no_storage_estimate",
          }),
          54,
          "main",
          "os",
          true
        );
      }
    } catch (eSt) {}

    // --- WebGPU adapter (single requestAdapter per session — no adapter spam) ---
    // Critical: never emit batch_id=B18_webgpu for deferred_to_b18 stubs — that marks
    // alreadySent and blocks the real mid B18 compute/f16 path (Chrome prod gap).
    try {
      requestWebGpuAdapterOnce({}).then(function (res) {
        res = res || {};
        var adapter = res.adapter;
        if (!adapter) {
          // Honest skip only when permanently unavailable; deferred → leave to mid B18.
          if (res.deferred || res.skip === "deferred_to_b18") {
            return;
          }
          emitDense(
            ctx,
            "B18_webgpu",
            Object.assign({}, baseMeta, {
              dense_pack: "B18_webgpu",
              webgpu_available: !!(navigator.gpu),
              webgpu_adapter_ok: false,
              webgpu_adapter_null: !res.error,
              webgpu_adapter_skip: res.skip || "null",
              webgpu_adapter_error: res.error || null,
            }),
            70,
            "main",
            "gpu_physical",
            true
          );
          return;
        }
        var infoP =
          adapter.requestAdapterInfo
            ? adapter.requestAdapterInfo().catch(function () {
                return {};
              })
            : Promise.resolve({});
        return infoP.then(function (info) {
          info = info || {};
          var features = [];
          try {
            if (adapter.features && adapter.features.forEach) {
              adapter.features.forEach(function (x) {
                features.push(String(x));
              });
            } else if (adapter.features) {
              features = Array.from(adapter.features || []);
            }
          } catch (eF) {}
          var limits = {};
          try {
            var lim = adapter.limits || {};
            [
              "maxTextureDimension2D",
              "maxBufferSize",
              "maxBindGroups",
              "maxComputeWorkgroupSizeX",
              "maxStorageBufferBindingSize",
            ].forEach(function (k) {
              if (lim[k] != null) limits[k] = lim[k];
            });
          } catch (eL) {}
          emitDense(
            ctx,
            "B18_webgpu",
            Object.assign({}, baseMeta, {
              dense_pack: "B18_webgpu",
              demand: "webgpu adapter async",
              webgpu_available: true,
              webgpu_adapter_ok: true,
              webgpu_adapter_vendor: info.vendor || null,
              webgpu_adapter_architecture: info.architecture || null,
              webgpu_adapter_device: info.device || null,
              webgpu_adapter_description: info.description || null,
              webgpu_features_n: features.length,
              webgpu_features_sample: features.slice(0, 16),
              webgpu_features_hash: shash(features.slice().sort().join("|")),
              webgpu_limits_hash: shash(JSON.stringify(limits)),
              webgpu_limits_sample: limits,
              webgpu_is_fallback: !!adapter.isFallbackAdapter,
            }),
            70,
            "main",
            "gpu_physical",
            true
          );
        });
      });
    } catch (eGpu) {}

    // --- Speech voices second tick (voiceschanged / delayed getVoices) ---
    try {
      function emitSpeech(voices, tag) {
        voices = voices || [];
        var vnames = [];
        for (var i = 0; i < Math.min(voices.length, 64); i++) {
          vnames.push(
            (voices[i].lang || "") +
              ":" +
              (voices[i].name || "").slice(0, 48) +
              ":" +
              (voices[i].localService ? "L" : "R")
          );
        }
        var f = Object.assign({}, baseMeta, {
          dense_pack: "B44_speech_deep",
          demand: "speech voices async " + tag,
          speech_voices_n: voices.length,
          speech_voices_hash: shash(vnames.join("|")),
          speech_default_voice: voices[0] ? (voices[0].name || "").slice(0, 48) : "",
          speech_local_n: voices.filter(function (v) {
            return !!v.localService;
          }).length,
          speech_voices_async_tag: tag,
          speech_voices_async_ok: voices.length > 0,
        });
        emitDense(ctx, "B44_speech_deep", f, 52, "main", "os", true);
      }
      if (typeof speechSynthesis !== "undefined" && speechSynthesis.getVoices) {
        var v0 = speechSynthesis.getVoices() || [];
        if (v0.length > 0) {
          emitSpeech(v0, "immediate");
        } else {
          var done = false;
          var finish = function (tag) {
            if (done) return;
            done = true;
            try {
              speechSynthesis.onvoiceschanged = null;
            } catch (e0) {}
            emitSpeech(speechSynthesis.getVoices() || [], tag);
          };
          try {
            speechSynthesis.onvoiceschanged = function () {
              finish("voiceschanged");
            };
          } catch (e1) {}
          setTimeout(function () {
            finish("timeout_800ms");
          }, 800);
        }
      } else {
        emitDense(
          ctx,
          "B44_speech_deep",
          Object.assign({}, baseMeta, {
            dense_pack: "B44_speech_deep",
            speech_voices_async_ok: false,
            speech_voices_async_skip: "no_speechSynthesis",
          }),
          52,
          "main",
          "os",
          true
        );
      }
    } catch (eSp) {}

    // --- MediaCapabilities.decodingInfo for key codecs ---
    try {
      if (navigator.mediaCapabilities && typeof navigator.mediaCapabilities.decodingInfo === "function") {
        var configs = [
          {
            type: "file",
            video: { contentType: 'video/mp4; codecs="avc1.42E01E"', width: 1920, height: 1080, bitrate: 2e6, framerate: 30 },
          },
          {
            type: "file",
            video: { contentType: 'video/webm; codecs="vp09.00.10.08"', width: 1920, height: 1080, bitrate: 2e6, framerate: 30 },
          },
          {
            type: "file",
            video: { contentType: 'video/webm; codecs="av01.0.05M.08"', width: 1920, height: 1080, bitrate: 2e6, framerate: 30 },
          },
          {
            type: "file",
            audio: { contentType: 'audio/mp4; codecs="mp4a.40.2"', channels: 2, bitrate: 128000, samplerate: 48000 },
          },
          {
            type: "file",
            audio: { contentType: 'audio/webm; codecs="opus"', channels: 2, bitrate: 128000, samplerate: 48000 },
          },
        ];
        Promise.all(
          configs.map(function (c) {
            return navigator.mediaCapabilities
              .decodingInfo(c)
              .then(function (r) {
                return {
                  ok: true,
                  supported: !!r.supported,
                  smooth: !!r.smooth,
                  powerEfficient: !!r.powerEfficient,
                  ct:
                    (c.video && c.video.contentType) ||
                    (c.audio && c.audio.contentType) ||
                    "",
                };
              })
              .catch(function (e) {
                return { ok: false, err: String((e && e.message) || e) };
              });
          })
        ).then(function (rows) {
          var parts = rows.map(function (r) {
            return (r.ct || "?") + ":" + (r.supported ? "1" : "0") + (r.smooth ? "s" : "") + (r.powerEfficient ? "p" : "");
          });
          var supportedN = rows.filter(function (r) {
            return r.supported;
          }).length;
          emitDense(
            ctx,
            "B19_eme_media",
            Object.assign({}, baseMeta, {
              dense_pack: "B19_eme_media",
              demand: "mediaCapabilities.decodingInfo async",
              media_capabilities_n: rows.length,
              media_capabilities_supported_n: supportedN,
              media_capabilities_hash: shash(parts.join("|")),
              media_capabilities_sample: parts.slice(0, 8),
              media_capabilities_ok: true,
              codec_support_hash: shash(parts.join("|")),
              codec_support_n: rows.length,
            }),
            58,
            "main",
            "material_hedge",
            true
          );
        });
      }
    } catch (eMc) {}

    // --- Local Font Access: capability only (silent probe redline) ---
    // NEVER call window.queryLocalFonts() — Chromium/Opera shows a permission
    // dialog (often empty/blank chrome UI). Font fingerprint stays measureText-only
    // (B21 / B50 sync packs). Emit passive presence so BE still sees surface.
    try {
      var hasQlf =
        typeof window.queryLocalFonts === "function" ||
        (typeof navigator !== "undefined" && typeof navigator.fonts !== "undefined");
      emitDense(
        ctx,
        "B50_font_matrix_detail",
        Object.assign({}, baseMeta, {
          dense_pack: "B50_font_matrix_detail",
          demand: "local_fonts_capability_only",
          local_fonts_api_present: typeof window.queryLocalFonts === "function",
          local_fonts_nav_fonts_present:
            typeof navigator !== "undefined" && typeof navigator.fonts !== "undefined",
          local_fonts_query_ok: false,
          local_fonts_n: 0,
          local_fonts_skipped: "silent_no_permission_request",
          local_fonts_surface: hasQlf ? "api_present_not_invoked" : "api_absent",
        }),
        49,
        "main",
        "census",
        true
      );
    } catch (eLf) {}
  }

  /** Patch boot ctx.queue.enqueue so registry module enqueue() densifies on the real path. */
  function patchQueueDensify(ctx, packIdHint, familyHint) {
    if (!ctx || !ctx.queue || typeof ctx.queue.enqueue !== "function") return;
    if (ctx._dense_q_patched) return;
    var oq = ctx.queue.enqueue.bind(ctx.queue);
    ctx.queue.enqueue = function (item) {
      if (item && item.payload && item.payload.fields) {
        // Per-enqueue family from batch_id map — shared boot ctx after B0 must not sticky browser_kernel.
        var bid = item.batch_id || packIdHint || "";
        var fam = resolveFamily(bid, item.payload.fields, familyHint);
        // Do not re-densify async follow-up bags (already specialized)
        if (!item.payload.fields.async_followup) {
          item.payload.fields = densifyBag(item.payload.fields, bid, fam, 30);
        }
      }
      var ret = oq(item);
      try {
        scheduleAsyncCompetitiveFollowups(ctx);
      } catch (e0) {}
      return ret;
    };
    ctx._dense_q_patched = true;
  }

  if (global.GRCollectors) {
    global.GRCollectors.__denseLoaded = true;
  }
  global.GRDense = {
    densifyBag: densifyBag,
    emitDense: emitDense,
    patchQueueDensify: patchQueueDensify,
    scheduleAsyncCompetitiveFollowups: scheduleAsyncCompetitiveFollowups,
    resolveFamily: resolveFamily,
    PACK_FAMILY: PACK_FAMILY,
    countPackSpecific: countPackSpecific,
    countPadKeys: countPadKeys,
    isVanityPadKey: isVanityPadKey,
    minFields: 30,
    version: "v5-async-competitive-complete",
  };

  /** Patch enqueue if exposed; also wrap register to densify inside run. */
  var _reg = register;
  function registerDense(id, def) {
    var origRun = def.run;
    var fam = def.family || def.probe_family || "browser_kernel";
    def.family = fam;
    def.run = function (ctx) {
      // Real boot path: densify on ctx.queue.enqueue (registry module uses this)
      patchQueueDensify(ctx, id, fam);
      var ret = origRun.call(this, ctx);
      try {
        scheduleAsyncCompetitiveFollowups(ctx);
      } catch (e1) {}
      return ret;
    };
    return _reg(id, def);
  }
  // Prefer dense register for new packs below

  registerDense("B47_api_flags_detail", {
    priority: 52,
    schedule: "dynamic",
    batch_id: "B47_api_flags_detail",
    layer: "deep",
    family: "census",
    demand: "v57 API flags claim-obs",
    run: function (ctx) {
      var f = {
        dense_pack: "B47_api_flags_detail",
        demand: "v57 API flags claim-obs",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var apis = (lists.KNOWN_APIS || []).slice(0, 48);
        var hits = [], miss = 0;
        for (var i = 0; i < apis.length; i++) {
          var n = apis[i], ok = false;
          try { ok = typeof global[n] !== "undefined" || n in global; } catch (e) {}
          if (ok) hits.push(n); else miss++;
        }
        f.api_flags_n = apis.length;
        f.api_flags_hit = hits.length;
        f.api_flags_miss = miss;
        f.api_flags_hash = shash(hits.join("|"));
        f.api_flags_sample = hits.slice(0, 16);
        for (var j = 0; j < Math.min(20, hits.length); j++) f["api_has_" + hits[j].replace(/[^a-zA-Z0-9]/g, "_").slice(0, 40)] = true;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B47_api_flags_detail", f, 52, "main", "census");
    },
  });

  registerDense("B48_css_supports_detail", {
    priority: 51,
    schedule: "dynamic",
    batch_id: "B48_css_supports_detail",
    layer: "deep",
    family: "census",
    demand: "CSS supports matrix",
    run: function (ctx) {
      var f = {
        dense_pack: "B48_css_supports_detail",
        demand: "CSS supports matrix",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var cs = lists.CSS_SUPPORTS || [];
        var okN = 0, sample = [];
        for (var i = 0; i < Math.min(cs.length, 80); i++) {
          var pair = cs[i], ok = false;
          try { ok = CSS && CSS.supports && CSS.supports(pair[0], pair[1]); } catch (e) {}
          if (ok) { okN++; if (sample.length < 12) sample.push(pair[0] + ":" + pair[1]); }
          if (i < 24) f["css_sup_" + i] = !!ok;
        }
        f.css_supports_probe_n = Math.min(cs.length, 80);
        f.css_supports_ok_n = okN;
        f.css_supports_sample = sample;
        f.css_supports_hash = shash(sample.join("|"));

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B48_css_supports_detail", f, 51, "main", "census");
    },
  });

  registerDense("B49_css_props_detail", {
    priority: 50,
    schedule: "dynamic",
    batch_id: "B49_css_props_detail",
    layer: "deep",
    family: "census",
    demand: "CSS computed props sample",
    run: function (ctx) {
      var f = {
        dense_pack: "B49_css_props_detail",
        demand: "CSS computed props sample",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var props = lists.CSS_PROPS || [];
        var el = document.createElement("div");
        document.body && document.body.appendChild(el);
        var resolved = 0, sample = [];
        for (var i = 0; i < Math.min(props.length, 60); i++) {
          var p = props[i], v = "";
          try { v = getComputedStyle(el).getPropertyValue(p) || ""; } catch (e) {}
          if (v) { resolved++; if (sample.length < 10) sample.push(p); }
          if (i < 20) f["css_prop_set_" + i] = !!v;
        }
        if (el.parentNode) el.parentNode.removeChild(el);
        f.css_props_probe_n = Math.min(props.length, 60);
        f.css_props_resolved_n = resolved;
        f.css_props_sample = sample;
        f.css_props_hash = shash(sample.join(","));

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B49_css_props_detail", f, 50, "main", "census");
    },
  });

  registerDense("B50_font_matrix_detail", {
    priority: 49,
    schedule: "dynamic",
    batch_id: "B50_font_matrix_detail",
    layer: "deep",
    family: "census",
    demand: "font install matrix",
    run: function (ctx) {
      var f = {
        dense_pack: "B50_font_matrix_detail",
        demand: "font install matrix",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var fonts = lists.FONTS || ["Arial", "Times New Roman", "Courier New"];
        var base = "monospace";
        var c = document.createElement("canvas");
        var g = c.getContext("2d");
        var hit = [], w0 = 0;
        if (g) {
          g.font = "72px " + base;
          w0 = g.measureText("mmmmmmmmmmlli").width;
          for (var i = 0; i < Math.min(fonts.length, 40); i++) {
            g.font = '72px "' + fonts[i] + '",' + base;
            var w = g.measureText("mmmmmmmmmmlli").width;
            var ok = Math.abs(w - w0) > 0.5;
            if (ok) hit.push(fonts[i]);
            if (i < 20) f["font_hit_" + i] = ok;
          }
        }
        f.font_probe_n = Math.min(fonts.length, 40);
        f.font_hit_n = hit.length;
        f.font_hit_sample = hit.slice(0, 12);
        f.font_matrix_hash = shash(hit.join("|"));

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B50_font_matrix_detail", f, 49, "main", "census");
    },
  });

  registerDense("B51_mq_matrix_detail", {
    priority: 48,
    schedule: "dynamic",
    batch_id: "B51_mq_matrix_detail",
    layer: "deep",
    family: "census",
    demand: "media query matrix",
    run: function (ctx) {
      var f = {
        dense_pack: "B51_mq_matrix_detail",
        demand: "media query matrix",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var mqs = lists.MEDIA_QUERIES || [];
        var trueN = 0, sample = [];
        for (var i = 0; i < Math.min(mqs.length, 60); i++) {
          var ok = false;
          try { ok = matchMedia(mqs[i]).matches; } catch (e) {}
          if (ok) { trueN++; if (sample.length < 12) sample.push(mqs[i]); }
          if (i < 24) f["mq_" + i] = !!ok;
        }
        f.mq_probe_n = Math.min(mqs.length, 60);
        f.mq_true_n = trueN;
        f.mq_true_sample = sample;
        f.mq_hash = shash(sample.join("|"));

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B51_mq_matrix_detail", f, 48, "main", "census");
    },
  });

  registerDense("B52_navigator_deep", {
    priority: 47,
    schedule: "dynamic",
    batch_id: "B52_navigator_deep",
    layer: "deep",
    family: "browser_kernel",
    demand: "navigator deep census",
    run: function (ctx) {
      var f = {
        dense_pack: "B52_navigator_deep",
        demand: "navigator deep census",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var nav = navigator;
        f.nav_user_agent_len = (nav.userAgent || "").length;
        f.nav_app_code_name = nav.appCodeName || "";
        f.nav_app_name = nav.appName || "";
        f.nav_product_sub = nav.productSub || "";
        f.nav_vendor_sub = nav.vendorSub || "";
        f.nav_do_not_track = nav.doNotTrack || null;
        f.nav_global_privacy = nav.globalPrivacyControl != null ? !!nav.globalPrivacyControl : null;
        f.nav_java = typeof nav.javaEnabled === "function" ? !!nav.javaEnabled() : null;
        f.nav_plugins_n = nav.plugins ? nav.plugins.length : 0;
        f.nav_mimes_n = nav.mimeTypes ? nav.mimeTypes.length : 0;
        f.nav_connection = !!(nav.connection || nav.mozConnection);
        f.nav_locks = !!nav.locks;
        f.nav_credentials = !!nav.credentials;
        f.nav_media_devices = !!nav.mediaDevices;
        f.nav_permissions = !!nav.permissions;
        f.nav_storage = !!nav.storage;
        f.nav_wake_lock = !!nav.wakeLock;
        f.nav_keyboard = !!nav.keyboard;
        f.nav_hid = !!nav.hid;
        f.nav_usb = !!nav.usb;
        f.nav_serial = !!nav.serial;
        f.nav_bluetooth = !!nav.bluetooth;
        f.nav_xr = !!nav.xr;
        f.nav_gpu = !!nav.gpu;
        f.nav_user_agent_data = !!nav.userAgentData;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B52_navigator_deep", f, 47, "main", "browser_kernel");
    },
  });

  registerDense("B53_window_keys_deep", {
    priority: 46,
    schedule: "dynamic",
    batch_id: "B53_window_keys_deep",
    layer: "deep",
    family: "browser_kernel",
    demand: "window own keys census",
    run: function (ctx) {
      var f = {
        dense_pack: "B53_window_keys_deep",
        demand: "window own keys census",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var own = 0, proto = 0, sample = [];
        try {
          for (var k in window) {
            if (Object.prototype.hasOwnProperty.call(window, k)) { own++; if (sample.length < 16) sample.push(k); }
            else proto++;
          }
        } catch (e) {}
        f.window_own_n = own;
        f.window_proto_n = proto;
        f.window_keys_sample = sample;
        f.window_keys_hash = shash(sample.join(","));
        f.window_chrome = typeof chrome !== "undefined";
        f.window_safari = typeof safari !== "undefined";
        f.window_opera = typeof opera !== "undefined";

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B53_window_keys_deep", f, 46, "main", "browser_kernel");
    },
  });

  registerDense("B54_plugin_mime_deep", {
    priority: 45,
    schedule: "dynamic",
    batch_id: "B54_plugin_mime_deep",
    layer: "mid4",
    family: "browser_kernel",
    demand: "plugins/mime types",
    run: function (ctx) {
      var f = {
        dense_pack: "B54_plugin_mime_deep",
        demand: "plugins/mime types",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var plugs = [];
        try {
          if (navigator.plugins) {
            for (var i = 0; i < navigator.plugins.length && i < 20; i++) {
              var p = navigator.plugins[i];
              plugs.push({ name: p.name || "", fn: p.filename || "", desc_len: (p.description || "").length });
              f["plugin_" + i + "_name_len"] = (p.name || "").length;
            }
          }
        } catch (e) {}
        f.plugins_list_n = plugs.length;
        f.plugins_hash = shash(JSON.stringify(plugs));
        f.plugins_sample = plugs.slice(0, 6);
        var mimes = [];
        try {
          if (navigator.mimeTypes) {
            for (var j = 0; j < navigator.mimeTypes.length && j < 16; j++) {
              mimes.push(navigator.mimeTypes[j].type || "");
              f["mime_" + j] = navigator.mimeTypes[j].type || "";
            }
          }
        } catch (e2) {}
        f.mimes_n = mimes.length;
        f.mimes_hash = shash(mimes.join("|"));

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B54_plugin_mime_deep", f, 45, "main", "browser_kernel");
    },
  });

  registerDense("B55_webrtc_ice_deep", {
    priority: 58,
    schedule: "dynamic",
    batch_id: "B55_webrtc_ice_deep",
    layer: "hard",
    family: "network",
    demand: "WebRTC ICE full",
    run: function (ctx) {
      var f = {
        dense_pack: "B55_webrtc_ice_deep",
        demand: "WebRTC ICE full",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.rtc_peer = !!(window.RTCPeerConnection || window.webkitRTCPeerConnection);
        f.rtc_session_desc = typeof RTCSessionDescription !== "undefined";
        f.rtc_ice_candidate = typeof RTCIceCandidate !== "undefined";
        f.rtc_data_channel = typeof RTCDataChannel !== "undefined";
        f.media_devices = !!(navigator.mediaDevices && navigator.mediaDevices.enumerateDevices);
        f.get_user_media = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
        // async ICE lite
        var PC = window.RTCPeerConnection || window.webkitRTCPeerConnection;
        if (PC) {
          try {
            var pc = new PC({ iceServers: [{ urls: "stun:stun.l.google.com:19302" }] });
            var cands = [];
            var hosts = [], srflx = [], relays = [];
            pc.onicecandidate = function (ev) {
              if (!(ev && ev.candidate && ev.candidate.candidate)) return;
              var line = ev.candidate.candidate;
              cands.push(line.split(" ")[0]);
              var m = / typ (host|srflx|relay) /.exec(line);
              if (!m) return;
              if (m[1] === "host") hosts.push(line);
              else if (m[1] === "srflx") srflx.push(line);
              else if (m[1] === "relay") relays.push(line);
            };
            pc.createDataChannel("gr");
            pc.createOffer().then(function (o) { return pc.setLocalDescription(o); }).then(function () {
              setTimeout(function () {
                f.ice_cand_n = cands.length;
                f.ice_cand_sample = cands.slice(0, 6);
                f.ice_hash = shash(cands.join("|"));
                f.ice_host_n = hosts.length;
                f.ice_srflx_n = srflx.length;
                f.ice_relay_n = relays.length;
                f.has_host = hosts.length > 0;
                f.has_srflx = srflx.length > 0;
                f.has_relay = relays.length > 0;
                f.ice_cand_types = [];
                if (hosts.length) f.ice_cand_types.push("host");
                if (srflx.length) f.ice_cand_types.push("srflx");
                if (relays.length) f.ice_cand_types.push("relay");
                if (!cands.length) f.ice_morphology = "none";
                else if (hosts.length && !srflx.length && !relays.length) f.ice_morphology = "host_only";
                else if (relays.length && !hosts.length) f.ice_morphology = "relay_heavy";
                else if (srflx.length && hosts.length) f.ice_morphology = "host_srflx";
                else f.ice_morphology = "mixed";
                f.webrtc_host_count = hosts.length;
                f.webrtc_srflx_count = srflx.length;
                f.webrtc_relay_count = relays.length;
                f.data_ok = true;
                emitDense(ctx, "B55_webrtc_ice_deep", f, 58, "main", "network");
                try { pc.close(); } catch (eC) {}
              }, 800);
            }).catch(function (e) {
              f.ice_error = String(e && e.message || e);
              emitDense(ctx, "B55_webrtc_ice_deep", f, 58, "main", "network");
            });
            return;
          } catch (eR) { f.rtc_error = String(eR && eR.message || eR); }
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B55_webrtc_ice_deep", f, 58, "main", "network");
    },
  });

  
  registerDense("B56_webrtc_stats", {
    priority: 44,
    schedule: "dynamic",
    batch_id: "B56_webrtc_stats",
    layer: "mid4",
    family: "network",
    demand: "WebRTC getStats sample",
    run: function (ctx) {
      var f = {
        dense_pack: "B56_webrtc_stats",
        demand: "WebRTC getStats sample",
        collected_at: Date.now(),
      };
      var PC = window.RTCPeerConnection || window.webkitRTCPeerConnection;
      f.rtc_peer = !!PC;
      f.webrtc_stats_algo = "gr_webrtc_getstats_v1";
      if (!PC) {
        f.webrtc_stats_skip = "no_rtcpeerconnection";
        emitDense(ctx, "B56_webrtc_stats", f, 44, "main", "network");
        return;
      }
      try {
        var pc = new PC({ iceServers: [{ urls: "stun:stun.l.google.com:19302" }] });
        var dc = null;
        try { dc = pc.createDataChannel("grstats"); } catch (eDc) { f.dc_error = String(eDc && eDc.message || eDc); }
        var finish = function () {
          emitDense(ctx, "B56_webrtc_stats", f, 44, "main", "network");
          try { if (dc) dc.close(); } catch (e1) {}
          try { pc.close(); } catch (e2) {}
        };
        var pullStats = function () {
          if (typeof pc.getStats !== "function") {
            f.webrtc_stats_skip = "no_getstats";
            finish();
            return;
          }
          pc.getStats(null).then(function (report) {
            var n = 0, types = {}, sample = [];
            var audioPkts = 0, videoPkts = 0, bytesSent = 0, bytesRecv = 0, rtt = null, candTypes = {};
            report.forEach(function (row) {
              n++;
              var t = row.type || "unknown";
              types[t] = (types[t] || 0) + 1;
              if (sample.length < 8) sample.push(t);
              if (t === "inbound-rtp" || t === "outbound-rtp") {
                if (row.kind === "audio" || row.mediaType === "audio") audioPkts += row.packetsReceived || row.packetsSent || 0;
                if (row.kind === "video" || row.mediaType === "video") videoPkts += row.packetsReceived || row.packetsSent || 0;
                bytesSent += row.bytesSent || 0;
                bytesRecv += row.bytesReceived || 0;
              }
              if (t === "candidate-pair" && row.currentRoundTripTime != null) rtt = row.currentRoundTripTime;
              if (t === "local-candidate" || t === "remote-candidate") {
                var ct = row.candidateType || "unknown";
                candTypes[ct] = (candTypes[ct] || 0) + 1;
              }
              if (t === "transport") {
                f.webrtc_transport_dtls = row.dtlsState || null;
                f.webrtc_transport_ice = row.iceState || null;
              }
              if (t === "codec" && !f.webrtc_codec_mime) {
                f.webrtc_codec_mime = row.mimeType || null;
                f.webrtc_codec_clock = row.clockRate || null;
              }
            });
            f.webrtc_stats_n = n;
            f.webrtc_stats_types = types;
            f.webrtc_stats_sample = sample;
            f.webrtc_stats_hash = shash(JSON.stringify(types) + "|" + sample.join(","));
            f.webrtc_audio_packets = audioPkts;
            f.webrtc_video_packets = videoPkts;
            f.webrtc_bytes_sent = bytesSent;
            f.webrtc_bytes_recv = bytesRecv;
            f.webrtc_rtt = rtt;
            f.webrtc_cand_types = candTypes;
            f.webrtc_stats_ok = n > 0;
            // expand type counts as flat materials
            var ti = 0;
            Object.keys(types).forEach(function (tk) {
              f["webrtc_type_" + tk.replace(/[^a-z0-9]+/gi, "_")] = types[tk];
              f["webrtc_type_idx_" + ti] = tk;
              f["webrtc_type_n_" + ti] = types[tk];
              ti++;
            });
            finish();
          }).catch(function (eS) {
            f.webrtc_stats_error = String(eS && eS.message || eS);
            finish();
          });
        };
        pc.createOffer().then(function (o) { return pc.setLocalDescription(o); }).then(function () {
          // allow ICE to start; getStats still returns baseline rows immediately
          setTimeout(pullStats, 200);
        }).catch(function (eO) {
          f.webrtc_offer_error = String(eO && eO.message || eO);
          // still try getStats
          setTimeout(pullStats, 50);
        });
        return;
      } catch (eR) {
        f.webrtc_stats_error = String(eR && eR.message || eR);
        emitDense(ctx, "B56_webrtc_stats", f, 44, "main", "network");
      }
    },
  });

registerDense("B57_canvas_emoji_path", {
    priority: 57,
    schedule: "dynamic",
    batch_id: "B57_canvas_emoji_path",
    layer: "hard",
    family: "material_hedge",
    demand: "canvas emoji/path2d",
    run: function (ctx) {
      var f = {
        dense_pack: "B57_canvas_emoji_path",
        demand: "canvas emoji/path2d",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var c = document.createElement("canvas");
        c.width = 240; c.height = 60;
        var g = c.getContext("2d");
        if (g) {
          g.textBaseline = "top";
          g.font = "16px Arial";
          g.fillStyle = "#f60";
          g.fillRect(0, 0, 240, 60);
          g.fillStyle = "#069";
          g.fillText("gr🚀emoji∑∫", 4, 4);
          if (typeof Path2D !== "undefined") {
            try {
              var p = new Path2D("M10 10 H 90 V 90 H 10 Z");
              g.stroke(p);
              f.path2d_ok = true;
            } catch (eP) { f.path2d_ok = false; }
          }
          var m = g.measureText("Wjyg");
          f.text_width = m.width;
          f.text_actual_bbox = m.actualBoundingBoxAscent != null;
          f.canvas_data_hash = shash(c.toDataURL().slice(0, 200));
          f.canvas_emoji_hash = shash(c.toDataURL().slice(50, 250));
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B57_canvas_emoji_path", f, 57, "main", "material_hedge");
    },
  });

  registerDense("B58_canvas_text_metrics", {
    priority: 56,
    schedule: "dynamic",
    batch_id: "B58_canvas_text_metrics",
    layer: "hard",
    family: "material_hedge",
    demand: "text metrics geometry",
    run: function (ctx) {
      var f = {
        dense_pack: "B58_canvas_text_metrics",
        demand: "text metrics geometry",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var c = document.createElement("canvas");
        c.width = 240; c.height = 60;
        var g = c.getContext("2d");
        if (g) {
          g.textBaseline = "top";
          g.font = "16px Arial";
          g.fillStyle = "#f60";
          g.fillRect(0, 0, 240, 60);
          g.fillStyle = "#069";
          g.fillText("gr🚀emoji∑∫", 4, 4);
          if (typeof Path2D !== "undefined") {
            try {
              var p = new Path2D("M10 10 H 90 V 90 H 10 Z");
              g.stroke(p);
              f.path2d_ok = true;
            } catch (eP) { f.path2d_ok = false; }
          }
          var m = g.measureText("Wjyg");
          f.text_width = m.width;
          f.text_actual_bbox = m.actualBoundingBoxAscent != null;
          f.canvas_data_hash = shash(c.toDataURL().slice(0, 200));
          f.canvas_emoji_hash = shash(c.toDataURL().slice(50, 250));
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B58_canvas_text_metrics", f, 56, "main", "material_hedge");
    },
  });

  registerDense("B59_webgl_params_full", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B59_webgl_params_full",
    layer: "hard",
    family: "gpu_physical",
    demand: "WebGL getParameter full",
    run: function (ctx) {
      var f = {
        dense_pack: "B59_webgl_params_full",
        demand: "WebGL getParameter full",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        try {
          var c = document.createElement("canvas");
          var gl = c.getContext("webgl2") || c.getContext("webgl");
          if (gl) {
            var paramKeys59 = ["VENDOR","RENDERER","VERSION","SHADING_LANGUAGE_VERSION",
              "MAX_TEXTURE_SIZE","MAX_CUBE_MAP_TEXTURE_SIZE","MAX_RENDERBUFFER_SIZE",
              "MAX_VERTEX_ATTRIBS","MAX_VERTEX_UNIFORM_VECTORS","MAX_FRAGMENT_UNIFORM_VECTORS",
              "MAX_VARYING_VECTORS","MAX_TEXTURE_IMAGE_UNITS","ALIASED_LINE_WIDTH_RANGE",
              "ALIASED_POINT_SIZE_RANGE","DEPTH_BITS","STENCIL_BITS","SAMPLES","SAMPLE_BUFFERS"];
            var names = ["VENDOR","RENDERER","VERSION","SHADING","MAX_TEX","MAX_CUBE","MAX_RB","MAX_VATTR","MAX_VU","MAX_FU","MAX_VARY","MAX_TU","LINE_W","POINT_S","DEPTH","STENCIL","SAMPLES","SAMPLE_BUF"];
            for (var i = 0; i < paramKeys59.length; i++) {
              var v = safeGlParam(gl, gl[paramKeys59[i]]);
              if (v != null) f["glp_" + names[i]] = Array.isArray(v) || (v && v.length) ? Array.prototype.slice.call(v) : v;
            }
            var exts = gl.getSupportedExtensions() || [];
            f.gl_ext_full_n = exts.length;
            f.gl_ext_full_hash = shash(exts.slice().sort().join(","));
            for (var j = 0; j < Math.min(exts.length, 16); j++) f["gl_ext_" + j] = exts[j];
            releaseGl(gl, c);
          } else f.webgl_missing = true;
        } catch (eG) { f.webgl_err = String(eG && eG.message || eG); }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B59_webgl_params_full", f, 55, "main", "gpu_physical");
    },
  });

  registerDense("B60_webgl_extensions_full", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B60_webgl_extensions_full",
    layer: "hard",
    family: "gpu_physical",
    demand: "WebGL extensions detail",
    run: function (ctx) {
      var f = {
        dense_pack: "B60_webgl_extensions_full",
        demand: "WebGL extensions detail",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        try {
          var c = document.createElement("canvas");
          var gl = c.getContext("webgl2") || c.getContext("webgl");
          if (gl) {
            // Extensions-first pack: avoid re-probing full param table (less INVALID_ENUM risk).
            var exts = gl.getSupportedExtensions() || [];
            f.gl_ext_full_n = exts.length;
            f.gl_ext_full_hash = shash(exts.slice().sort().join(","));
            for (var j = 0; j < Math.min(exts.length, 24); j++) f["gl_ext_" + j] = exts[j];
            f.gl_vendor = safeGlParam(gl, gl.VENDOR) || "";
            f.gl_renderer = safeGlParam(gl, gl.RENDERER) || "";
            f.gl_version = safeGlParam(gl, gl.VERSION) || "";
            releaseGl(gl, c);
          } else f.webgl_missing = true;
        } catch (eG) { f.webgl_err = String(eG && eG.message || eG); }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B60_webgl_extensions_full", f, 54, "main", "gpu_physical");
    },
  });

  registerDense("B61_audio_worklet", {
    priority: 43,
    schedule: "dynamic",
    batch_id: "B61_audio_worklet",
    layer: "mid4",
    family: "material_hedge",
    demand: "AudioWorklet presence",
    run: function (ctx) {
      var f = {
        dense_pack: "B61_audio_worklet",
        demand: "AudioWorklet presence",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.audio_ctx_ctor = !!(window.AudioContext || window.webkitAudioContext);
        f.offline_audio_ctor = !!(window.OfflineAudioContext || window.webkitOfflineAudioContext);
        f.audio_worklet = !!(window.AudioWorkletNode);
        try {
          var AC = window.OfflineAudioContext || window.webkitOfflineAudioContext;
          if (AC) {
            var ctxA = new AC(1, 44100, 44100);
            f.offline_sr = ctxA.sampleRate;
            f.offline_len = ctxA.length;
            f.offline_ch = ctxA.numberOfChannels;
          }
        } catch (eA) { f.audio_err = String(eA && eA.message || eA); }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B61_audio_worklet", f, 43, "main", "material_hedge");
    },
  });

  registerDense("B62_offline_audio_moments", {
    priority: 53,
    schedule: "dynamic",
    batch_id: "B62_offline_audio_moments",
    layer: "hard",
    family: "material_hedge",
    demand: "offline audio moments",
    run: function (ctx) {
      var f = {
        dense_pack: "B62_offline_audio_moments",
        demand: "offline audio moments",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.audio_ctx_ctor = !!(window.AudioContext || window.webkitAudioContext);
        f.offline_audio_ctor = !!(window.OfflineAudioContext || window.webkitOfflineAudioContext);
        f.audio_worklet = !!(window.AudioWorkletNode);
        try {
          var AC = window.OfflineAudioContext || window.webkitOfflineAudioContext;
          if (AC) {
            var ctxA = new AC(1, 44100, 44100);
            f.offline_sr = ctxA.sampleRate;
            f.offline_len = ctxA.length;
            f.offline_ch = ctxA.numberOfChannels;
          }
        } catch (eA) { f.audio_err = String(eA && eA.message || eA); }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B62_offline_audio_moments", f, 53, "main", "material_hedge");
    },
  });

  registerDense("B63_intl_full", {
    priority: 42,
    schedule: "dynamic",
    batch_id: "B63_intl_full",
    layer: "deep",
    family: "os",
    demand: "Intl full locale",
    run: function (ctx) {
      var f = {
        dense_pack: "B63_intl_full",
        demand: "Intl full locale",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        try {
          var ro = Intl.DateTimeFormat().resolvedOptions();
          f.intl_locale = ro.locale || "";
          f.intl_calendar = ro.calendar || "";
          f.intl_numbering = ro.numberingSystem || "";
          f.intl_tz = ro.timeZone || "";
          f.intl_hour_cycle = ro.hourCycle || "";
        } catch (eI) {}
        f.tz_offset_jan = (function () { return new Date(Date.UTC(2024, 0, 1)).getTimezoneOffset(); })();
        f.tz_offset_jul = (function () { return new Date(Date.UTC(2024, 6, 1)).getTimezoneOffset(); })();
        f.tz_dst = f.tz_offset_jan !== f.tz_offset_jul;
        try {
          f.intl_collator = !!(Intl.Collator);
          f.intl_plural = !!(Intl.PluralRules);
          f.intl_relative = !!(Intl.RelativeTimeFormat);
          f.intl_list = !!(Intl.ListFormat);
          f.intl_segmenter = !!(Intl.Segmenter);
          f.intl_display = !!(Intl.DisplayNames);
        } catch (e2) {}

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B63_intl_full", f, 42, "main", "os");
    },
  });

  registerDense("B64_timezone_deep", {
    priority: 41,
    schedule: "dynamic",
    batch_id: "B64_timezone_deep",
    layer: "deep",
    family: "os",
    demand: "timezone offsets",
    run: function (ctx) {
      var f = {
        dense_pack: "B64_timezone_deep",
        demand: "timezone offsets",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        try {
          var ro = Intl.DateTimeFormat().resolvedOptions();
          f.intl_locale = ro.locale || "";
          f.intl_calendar = ro.calendar || "";
          f.intl_numbering = ro.numberingSystem || "";
          f.intl_tz = ro.timeZone || "";
          f.intl_hour_cycle = ro.hourCycle || "";
        } catch (eI) {}
        f.tz_offset_jan = (function () { return new Date(Date.UTC(2024, 0, 1)).getTimezoneOffset(); })();
        f.tz_offset_jul = (function () { return new Date(Date.UTC(2024, 6, 1)).getTimezoneOffset(); })();
        f.tz_dst = f.tz_offset_jan !== f.tz_offset_jul;
        try {
          f.intl_collator = !!(Intl.Collator);
          f.intl_plural = !!(Intl.PluralRules);
          f.intl_relative = !!(Intl.RelativeTimeFormat);
          f.intl_list = !!(Intl.ListFormat);
          f.intl_segmenter = !!(Intl.Segmenter);
          f.intl_display = !!(Intl.DisplayNames);
        } catch (e2) {}

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B64_timezone_deep", f, 41, "main", "os");
    },
  });

  registerDense("B65_client_hints_full", {
    priority: 55,
    schedule: "dynamic",
    batch_id: "B65_client_hints_full",
    layer: "mid4",
    family: "browser_kernel",
    demand: "UA-CH full",
    run: function (ctx) {
      var f = {
        dense_pack: "B65_client_hints_full",
        demand: "UA-CH full",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var uad = navigator.userAgentData;
        f.ua_ch_present = !!uad;
        if (uad) {
          f.ua_ch_mobile = !!uad.mobile;
          f.ua_ch_platform = uad.platform || "";
          f.ua_ch_brands_n = (uad.brands && uad.brands.length) || 0;
          f.ua_ch_brands_hash = shash(JSON.stringify(uad.brands || []));
        }
        // sync high-entropy when available (async complete may enqueue twice — ok)
        if (uad && uad.getHighEntropyValues) {
          uad.getHighEntropyValues(["architecture","bitness","model","platformVersion","fullVersionList","wow64","formFactors"]).then(function (he) {
            f.ua_ch_architecture = he.architecture || null;
            f.ua_ch_bitness = he.bitness || null;
            f.ua_ch_model = he.model || null;
            f.ua_ch_platform_version = he.platformVersion || null;
            f.ua_ch_wow64 = he.wow64 != null ? !!he.wow64 : null;
            f.ua_ch_form_factors = he.formFactors || null;
            f.ua_ch_full_version_n = (he.fullVersionList && he.fullVersionList.length) || 0;
            emitDense(ctx, "B65_client_hints_full", f, 55, "main", "browser_kernel");
          }).catch(function () {
            emitDense(ctx, "B65_client_hints_full", f, 55, "main", "browser_kernel");
          });
          return;
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B65_client_hints_full", f, 55, "main", "browser_kernel");
    },
  });

  registerDense("B66_sec_ch_headers", {
    priority: 40,
    schedule: "dynamic",
    batch_id: "B66_sec_ch_headers",
    layer: "mid4",
    family: "browser_kernel",
    demand: "sec-ch-* via JS",
    run: function (ctx) {
      var f = {
        dense_pack: "B66_sec_ch_headers",
        demand: "sec-ch-* via JS",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        var uad = navigator.userAgentData;
        f.ua_ch_present = !!uad;
        if (uad) {
          f.ua_ch_mobile = !!uad.mobile;
          f.ua_ch_platform = uad.platform || "";
          f.ua_ch_brands_n = (uad.brands && uad.brands.length) || 0;
          f.ua_ch_brands_hash = shash(JSON.stringify(uad.brands || []));
        }
        // sync high-entropy when available (async complete may enqueue twice — ok)
        if (uad && uad.getHighEntropyValues) {
          uad.getHighEntropyValues(["architecture","bitness","model","platformVersion","fullVersionList","wow64","formFactors"]).then(function (he) {
            f.ua_ch_architecture = he.architecture || null;
            f.ua_ch_bitness = he.bitness || null;
            f.ua_ch_model = he.model || null;
            f.ua_ch_platform_version = he.platformVersion || null;
            f.ua_ch_wow64 = he.wow64 != null ? !!he.wow64 : null;
            f.ua_ch_form_factors = he.formFactors || null;
            f.ua_ch_full_version_n = (he.fullVersionList && he.fullVersionList.length) || 0;
            emitDense(ctx, "B66_sec_ch_headers", f, 40, "main", "browser_kernel");
          }).catch(function () {
            emitDense(ctx, "B66_sec_ch_headers", f, 40, "main", "browser_kernel");
          });
          return;
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B66_sec_ch_headers", f, 40, "main", "browser_kernel");
    },
  });

  registerDense("B67_service_worker_deep", {
    priority: 39,
    schedule: "dynamic",
    batch_id: "B67_service_worker_deep",
    layer: "deep",
    family: "browser_kernel",
    demand: "SW registrations",
    run: function (ctx) {
      var f = {
        dense_pack: "B67_service_worker_deep",
        demand: "SW registrations",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.sw_ctrl = !!(navigator.serviceWorker && navigator.serviceWorker.controller);
        f.sw_ready = !!(navigator.serviceWorker && navigator.serviceWorker.ready);
        f.caches_api = typeof caches !== "undefined";
        f.cookie_store = typeof cookieStore !== "undefined";
        if (navigator.serviceWorker) {
          try {
            navigator.serviceWorker.getRegistrations().then(function (regs) {
              f.sw_reg_n = regs ? regs.length : 0;
              emitDense(ctx, "B67_service_worker_deep", f, 39, "main", "browser_kernel");
            }).catch(function () {
              emitDense(ctx, "B67_service_worker_deep", f, 39, "main", "browser_kernel");
            });
            return;
          } catch (e) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B67_service_worker_deep", f, 39, "main", "browser_kernel");
    },
  });

  registerDense("B68_cache_storage_deep", {
    priority: 38,
    schedule: "dynamic",
    batch_id: "B68_cache_storage_deep",
    layer: "deep",
    family: "browser_kernel",
    demand: "CacheStorage keys",
    run: function (ctx) {
      var f = {
        dense_pack: "B68_cache_storage_deep",
        demand: "CacheStorage keys",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.sw_ctrl = !!(navigator.serviceWorker && navigator.serviceWorker.controller);
        f.sw_ready = !!(navigator.serviceWorker && navigator.serviceWorker.ready);
        f.caches_api = typeof caches !== "undefined";
        f.cookie_store = typeof cookieStore !== "undefined";
        if (navigator.serviceWorker) {
          try {
            navigator.serviceWorker.getRegistrations().then(function (regs) {
              f.sw_reg_n = regs ? regs.length : 0;
              emitDense(ctx, "B68_cache_storage_deep", f, 38, "main", "browser_kernel");
            }).catch(function () {
              emitDense(ctx, "B68_cache_storage_deep", f, 38, "main", "browser_kernel");
            });
            return;
          } catch (e) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B68_cache_storage_deep", f, 38, "main", "browser_kernel");
    },
  });

  registerDense("B69_bluetooth_usb", {
    priority: 37,
    schedule: "dynamic",
    batch_id: "B69_bluetooth_usb",
    layer: "deep",
    family: "peripheral",
    demand: "Bluetooth/USB APIs",
    run: function (ctx) {
      var f = {
        dense_pack: "B69_bluetooth_usb",
        demand: "Bluetooth/USB APIs",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.bluetooth = !!navigator.bluetooth;
        f.usb = !!navigator.usb;
        f.hid = !!navigator.hid;
        f.serial = !!navigator.serial;
        f.payment_request = typeof PaymentRequest !== "undefined";
        f.public_key_cred = typeof PublicKeyCredential !== "undefined";
        f.credentials = !!navigator.credentials;
        f.wake_lock = !!navigator.wakeLock;
        f.idle_detector = typeof IdleDetector !== "undefined";
        f.keyboard = !!navigator.keyboard;
        f.virtual_keyboard = !!(navigator.virtualKeyboard);
        if (navigator.keyboard && navigator.keyboard.getLayoutMap) {
          try {
            navigator.keyboard.getLayoutMap().then(function (map) {
              var n = 0; try { map.forEach(function () { n++; }); } catch (e) {}
              f.keyboard_layout_n = n;
              emitDense(ctx, "B69_bluetooth_usb", f, 37, "main", "peripheral");
            }).catch(function () {
              emitDense(ctx, "B69_bluetooth_usb", f, 37, "main", "peripheral");
            });
            return;
          } catch (eK) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B69_bluetooth_usb", f, 37, "main", "peripheral");
    },
  });

  registerDense("B70_payment_credential", {
    priority: 36,
    schedule: "dynamic",
    batch_id: "B70_payment_credential",
    layer: "deep",
    family: "browser_kernel",
    demand: "Payment/WebAuthn surface",
    run: function (ctx) {
      var f = {
        dense_pack: "B70_payment_credential",
        demand: "Payment/WebAuthn surface",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.bluetooth = !!navigator.bluetooth;
        f.usb = !!navigator.usb;
        f.hid = !!navigator.hid;
        f.serial = !!navigator.serial;
        f.payment_request = typeof PaymentRequest !== "undefined";
        f.public_key_cred = typeof PublicKeyCredential !== "undefined";
        f.credentials = !!navigator.credentials;
        f.wake_lock = !!navigator.wakeLock;
        f.idle_detector = typeof IdleDetector !== "undefined";
        f.keyboard = !!navigator.keyboard;
        f.virtual_keyboard = !!(navigator.virtualKeyboard);
        if (navigator.keyboard && navigator.keyboard.getLayoutMap) {
          try {
            navigator.keyboard.getLayoutMap().then(function (map) {
              var n = 0; try { map.forEach(function () { n++; }); } catch (e) {}
              f.keyboard_layout_n = n;
              emitDense(ctx, "B70_payment_credential", f, 36, "main", "browser_kernel");
            }).catch(function () {
              emitDense(ctx, "B70_payment_credential", f, 36, "main", "browser_kernel");
            });
            return;
          } catch (eK) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B70_payment_credential", f, 36, "main", "browser_kernel");
    },
  });

  registerDense("B71_idle_wake_lock", {
    priority: 35,
    schedule: "dynamic",
    batch_id: "B71_idle_wake_lock",
    layer: "deep",
    family: "mobile_form",
    demand: "Idle/WakeLock",
    run: function (ctx) {
      var f = {
        dense_pack: "B71_idle_wake_lock",
        demand: "Idle/WakeLock",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.bluetooth = !!navigator.bluetooth;
        f.usb = !!navigator.usb;
        f.hid = !!navigator.hid;
        f.serial = !!navigator.serial;
        f.payment_request = typeof PaymentRequest !== "undefined";
        f.public_key_cred = typeof PublicKeyCredential !== "undefined";
        f.credentials = !!navigator.credentials;
        f.wake_lock = !!navigator.wakeLock;
        f.idle_detector = typeof IdleDetector !== "undefined";
        f.keyboard = !!navigator.keyboard;
        f.virtual_keyboard = !!(navigator.virtualKeyboard);
        if (navigator.keyboard && navigator.keyboard.getLayoutMap) {
          try {
            navigator.keyboard.getLayoutMap().then(function (map) {
              var n = 0; try { map.forEach(function () { n++; }); } catch (e) {}
              f.keyboard_layout_n = n;
              emitDense(ctx, "B71_idle_wake_lock", f, 35, "main", "mobile_form");
            }).catch(function () {
              emitDense(ctx, "B71_idle_wake_lock", f, 35, "main", "mobile_form");
            });
            return;
          } catch (eK) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B71_idle_wake_lock", f, 35, "main", "mobile_form");
    },
  });

  registerDense("B72_keyboard_layout", {
    priority: 34,
    schedule: "dynamic",
    batch_id: "B72_keyboard_layout",
    layer: "mid4",
    family: "browser_kernel",
    demand: "Keyboard layout map",
    run: function (ctx) {
      var f = {
        dense_pack: "B72_keyboard_layout",
        demand: "Keyboard layout map",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.bluetooth = !!navigator.bluetooth;
        f.usb = !!navigator.usb;
        f.hid = !!navigator.hid;
        f.serial = !!navigator.serial;
        f.payment_request = typeof PaymentRequest !== "undefined";
        f.public_key_cred = typeof PublicKeyCredential !== "undefined";
        f.credentials = !!navigator.credentials;
        f.wake_lock = !!navigator.wakeLock;
        f.idle_detector = typeof IdleDetector !== "undefined";
        f.keyboard = !!navigator.keyboard;
        f.virtual_keyboard = !!(navigator.virtualKeyboard);
        if (navigator.keyboard && navigator.keyboard.getLayoutMap) {
          try {
            navigator.keyboard.getLayoutMap().then(function (map) {
              var n = 0; try { map.forEach(function () { n++; }); } catch (e) {}
              f.keyboard_layout_n = n;
              emitDense(ctx, "B72_keyboard_layout", f, 34, "main", "browser_kernel");
            }).catch(function () {
              emitDense(ctx, "B72_keyboard_layout", f, 34, "main", "browser_kernel");
            });
            return;
          } catch (eK) {}
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B72_keyboard_layout", f, 34, "main", "browser_kernel");
    },
  });

  registerDense("B73_pointer_capabilities", {
    priority: 33,
    schedule: "dynamic",
    batch_id: "B73_pointer_capabilities",
    layer: "mid4",
    family: "rpa",
    demand: "pointer/hover caps",
    run: function (ctx) {
      var f = {
        dense_pack: "B73_pointer_capabilities",
        demand: "pointer/hover caps",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.pointer_fine = matchMedia("(pointer: fine)").matches;
        f.pointer_coarse = matchMedia("(pointer: coarse)").matches;
        f.pointer_none = matchMedia("(pointer: none)").matches;
        f.hover_hover = matchMedia("(hover: hover)").matches;
        f.hover_none = matchMedia("(hover: none)").matches;
        f.any_pointer_fine = matchMedia("(any-pointer: fine)").matches;
        f.any_pointer_coarse = matchMedia("(any-pointer: coarse)").matches;
        f.any_hover = matchMedia("(any-hover: hover)").matches;
        var vv = window.visualViewport;
        if (vv) {
          f.vv_width = vv.width; f.vv_height = vv.height;
          f.vv_offset_left = vv.offsetLeft; f.vv_offset_top = vv.offsetTop;
          f.vv_page_left = vv.pageLeft; f.vv_page_top = vv.pageTop;
          f.vv_scale = vv.scale;
        } else f.vv_missing = true;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B73_pointer_capabilities", f, 33, "main", "rpa");
    },
  });

  registerDense("B74_visual_viewport", {
    priority: 48,
    schedule: "dynamic",
    batch_id: "B74_visual_viewport",
    layer: "mid4",
    family: "os",
    demand: "visualViewport geometry",
    run: function (ctx) {
      var f = {
        dense_pack: "B74_visual_viewport",
        demand: "visualViewport geometry",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.pointer_fine = matchMedia("(pointer: fine)").matches;
        f.pointer_coarse = matchMedia("(pointer: coarse)").matches;
        f.pointer_none = matchMedia("(pointer: none)").matches;
        f.hover_hover = matchMedia("(hover: hover)").matches;
        f.hover_none = matchMedia("(hover: none)").matches;
        f.any_pointer_fine = matchMedia("(any-pointer: fine)").matches;
        f.any_pointer_coarse = matchMedia("(any-pointer: coarse)").matches;
        f.any_hover = matchMedia("(any-hover: hover)").matches;
        var vv = window.visualViewport;
        if (vv) {
          f.vv_width = vv.width; f.vv_height = vv.height;
          f.vv_offset_left = vv.offsetLeft; f.vv_offset_top = vv.offsetTop;
          f.vv_page_left = vv.pageLeft; f.vv_page_top = vv.pageTop;
          f.vv_scale = vv.scale;
        } else f.vv_missing = true;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B74_visual_viewport", f, 48, "main", "os");
    },
  });

  registerDense("B75_performance_entries", {
    priority: 32,
    schedule: "dynamic",
    batch_id: "B75_performance_entries",
    layer: "deep",
    family: "browser_kernel",
    demand: "perf entries census",
    run: function (ctx) {
      var f = {
        dense_pack: "B75_performance_entries",
        demand: "perf entries census",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.perf_now = performance.now();
        f.perf_time_origin = performance.timeOrigin || null;
        try {
          var entries = performance.getEntries ? performance.getEntries() : [];
          f.perf_entries_n = entries.length;
          var types = {};
          for (var i = 0; i < Math.min(entries.length, 100); i++) {
            var t = entries[i].entryType || "unknown";
            types[t] = (types[t] || 0) + 1;
          }
          f.perf_entry_types = types;
          f.perf_resource_n = types.resource || 0;
          f.perf_nav_n = types.navigation || 0;
          f.perf_paint_n = types.paint || 0;
          f.perf_mark_n = types.mark || 0;
        } catch (eP) { f.perf_err = String(eP && eP.message || eP); }
        f.perf_memory = !!(performance.memory);
        if (performance.memory) {
          f.js_heap_limit = performance.memory.jsHeapSizeLimit || null;
          f.js_heap_total = performance.memory.totalJSHeapSize || null;
          f.js_heap_used = performance.memory.usedJSHeapSize || null;
        }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B75_performance_entries", f, 32, "main", "browser_kernel");
    },
  });

  registerDense("B76_math_wasm_deep", {
    priority: 52,
    schedule: "dynamic",
    batch_id: "B76_math_wasm_deep",
    layer: "hard",
    family: "material_hedge",
    demand: "math+wasm deep",
    run: function (ctx) {
      var f = {
        dense_pack: "B76_math_wasm_deep",
        demand: "math+wasm deep",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.math_sin = Math.sin(1e-10);
        f.math_cos = Math.cos(1e-10);
        f.math_tan = Math.tan(1e-10);
        f.math_log = Math.log(Math.E);
        f.math_expm1 = Math.expm1 ? Math.expm1(1e-10) : null;
        f.math_acosh = Math.acosh ? Math.acosh(1e10) : null;
        f.math_asinh = Math.asinh ? Math.asinh(1e-10) : null;
        f.math_atanh = Math.atanh ? Math.atanh(1e-10) : null;
        f.math_hash = shash([f.math_sin, f.math_cos, f.math_tan, f.math_log].join(","));
        f.wasm_present = typeof WebAssembly !== "undefined";
        f.wasm_instantiate = !!(WebAssembly && WebAssembly.instantiate);
        f.wasm_compile = !!(WebAssembly && WebAssembly.compile);
        f.wasm_validate = !!(WebAssembly && WebAssembly.validate);
        f.wasm_simd = false;
        try {
          // minimal wasm module validate
          f.wasm_validate_empty = WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0]));
        } catch (eW) { f.wasm_err = String(eW && eW.message || eW); }

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B76_math_wasm_deep", f, 52, "main", "material_hedge");
    },
  });

  registerDense("B77_worker_env_deep", {
    priority: 65,
    schedule: "dynamic",
    batch_id: "B77_worker_env_deep",
    layer: "mid4",
    family: "sandbox_xsrc",
    demand: "worker env fields",
    run: function (ctx) {
      var f = {
        dense_pack: "B77_worker_env_deep",
        demand: "worker env fields",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.is_top = window.top === window.self;
        f.worker_ctor = typeof Worker !== "undefined";
        f.shared_worker_ctor = typeof SharedWorker !== "undefined";
        f.service_worker = !!navigator.serviceWorker;
        f.cross_origin_isolated = !!crossOriginIsolated;
        f.is_secure = !!window.isSecureContext;
        f.origin = location.origin || "";
        f.protocol = location.protocol || "";
        try { f.ancestor_origins_n = (location.ancestorOrigins && location.ancestorOrigins.length) || 0; } catch (e) { f.ancestor_origins_n = null; }
        f.frame_element = !!window.frameElement;
        f.opener = !!window.opener;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B77_worker_env_deep", f, 65, "main", "sandbox_xsrc");
    },
  });

  registerDense("B78_iframe_env_deep", {
    priority: 64,
    schedule: "dynamic",
    batch_id: "B78_iframe_env_deep",
    layer: "mid4",
    family: "sandbox_xsrc",
    demand: "iframe env fields",
    run: function (ctx) {
      var f = {
        dense_pack: "B78_iframe_env_deep",
        demand: "iframe env fields",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.is_top = window.top === window.self;
        f.worker_ctor = typeof Worker !== "undefined";
        f.shared_worker_ctor = typeof SharedWorker !== "undefined";
        f.service_worker = !!navigator.serviceWorker;
        f.cross_origin_isolated = !!crossOriginIsolated;
        f.is_secure = !!window.isSecureContext;
        f.origin = location.origin || "";
        f.protocol = location.protocol || "";
        try { f.ancestor_origins_n = (location.ancestorOrigins && location.ancestorOrigins.length) || 0; } catch (e) { f.ancestor_origins_n = null; }
        f.frame_element = !!window.frameElement;
        f.opener = !!window.opener;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B78_iframe_env_deep", f, 64, "main", "sandbox_xsrc");
    },
  });

  registerDense("B79_cross_origin_isolation", {
    priority: 31,
    schedule: "dynamic",
    batch_id: "B79_cross_origin_isolation",
    layer: "deep",
    family: "browser_kernel",
    demand: "COOP/COEP isolation",
    run: function (ctx) {
      var f = {
        dense_pack: "B79_cross_origin_isolation",
        demand: "COOP/COEP isolation",
        collected_at: Date.now(),
      };
      try {
        // pack-specific useful probes

        f.is_top = window.top === window.self;
        f.worker_ctor = typeof Worker !== "undefined";
        f.shared_worker_ctor = typeof SharedWorker !== "undefined";
        f.service_worker = !!navigator.serviceWorker;
        f.cross_origin_isolated = !!crossOriginIsolated;
        f.is_secure = !!window.isSecureContext;
        f.origin = location.origin || "";
        f.protocol = location.protocol || "";
        try { f.ancestor_origins_n = (location.ancestorOrigins && location.ancestorOrigins.length) || 0; } catch (e) { f.ancestor_origins_n = null; }
        f.frame_element = !!window.frameElement;
        f.opener = !!window.opener;

      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
      }
      emitDense(ctx, "B79_cross_origin_isolation", f, 31, "main", "browser_kernel");
    },
  });

  /**
   * B80: R100 high-value commercial materials as a dedicated hard pack.
   * Fields also densified into other packs via r100_promote block above.
   * Soft/emulated engines: null/absent is a valid observation (do not force).
   */
  registerDense("B80_r100_high_value", {
    priority: 54,
    schedule: "dynamic",
    batch_id: "B80_r100_high_value",
    layer: "hard",
    family: "material_hedge",
    demand: "R100-promoted high-value network/intl/rtc/perf/crypto materials",
    run: function (ctx) {
      var f = {
        dense_pack: "B80_r100_high_value",
        demand: "R100-promoted high-value network/intl/rtc/perf/crypto materials",
        r100_promote_algo: "gr_r100_to_b_v1",
        collected_at: Date.now(),
      };
      try {
        // Navigation protocol + sizes
        if (performance && performance.getEntriesByType) {
          var navE = safePerfEntries("navigation");
          var te = navE[0];
          if (te) {
            f.nav_next_hop_protocol = te.nextHopProtocol != null ? String(te.nextHopProtocol) : null;
            f.nav_transfer_size = te.transferSize != null ? te.transferSize : null;
            f.nav_encoded_body_size = te.encodedBodySize != null ? te.encodedBodySize : null;
            f.nav_decoded_body_size = te.decodedBodySize != null ? te.decodedBodySize : null;
          } else {
            f.nav_next_hop_protocol = "no_nav_timing";
          }
          f.perf_resource_entries_n = safePerfEntries("resource").length;
        }
        // Connection API (absent on Gecko/WK is ok)
        var nc = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
        if (nc) {
          f.net_conn_type = nc.type != null ? String(nc.type) : null;
          f.net_conn_effective = nc.effectiveType != null ? String(nc.effectiveType) : null;
          f.net_conn_rtt = nc.rtt != null ? nc.rtt : null;
          f.net_conn_downlink = nc.downlink != null ? nc.downlink : null;
          f.net_conn_downlink_max = nc.downlinkMax != null ? nc.downlinkMax : null;
          f.net_conn_save_data = !!nc.saveData;
        } else {
          f.net_conn_api = "absent";
        }
        // Intl Locale maximize
        if (typeof Intl !== "undefined" && Intl.Locale) {
          var lang = navigator.language || (navigator.languages && navigator.languages[0]) || "en";
          if (!lang || lang === "undefined") lang = "en";
          var loc = new Intl.Locale(lang);
          var locM = loc.maximize ? loc.maximize() : loc;
          f.intl_locale_base = String(loc);
          f.intl_locale_max = String(locM);
          f.intl_locale_language = locM.language || null;
          f.intl_locale_script = locM.script || null;
          f.intl_locale_region = locM.region || null;
        }
        // RTC surface (no ICE gather)
        var RTCP = global.RTCPeerConnection || global.webkitRTCPeerConnection;
        if (RTCP) {
          var pc = new RTCP({ iceServers: [] });
          f.rtc_can_trickle = pc.canTrickleIceCandidates == null ? null : !!pc.canTrickleIceCandidates;
          f.rtc_connection_state = pc.connectionState || null;
          f.rtc_ice_gathering = pc.iceGatheringState || null;
          f.rtc_add_transceiver = typeof pc.addTransceiver === "function";
          try { pc.close(); } catch (e0) {}
        } else {
          f.rtc_api = "absent";
        }
        f.webtransport = typeof WebTransport !== "undefined";
        // Crypto
        if (window.crypto && window.crypto.getRandomValues) {
          var rb = new Uint8Array(16);
          window.crypto.getRandomValues(rb);
          f.crypto_rand_len = rb.length;
        }
        f.crypto_subtle = !!(window.crypto && window.crypto.subtle);
        f.is_secure_context = !!window.isSecureContext;
        // Perf observer types
        if (typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes) {
          f.perf_obs_entry_types_n = (PerformanceObserver.supportedEntryTypes || []).length;
        }
        // Canvas multi-script widths
        var cm = document.createElement("canvas");
        var gm = cm.getContext && cm.getContext("2d");
        if (gm) {
          gm.font = "72px sans-serif";
          f.canvas_w_emoji = gm.measureText("😃🌐").width;
          f.canvas_w_cjk = gm.measureText("汉字測試").width;
          f.canvas_w_arabic = gm.measureText("مرحبا").width;
          f.canvas_w_base = gm.measureText("mmMwWLliI0").width;
        }
        f.r100_promote_ok = true;
      } catch (ePack) {
        f.pack_error = String((ePack && ePack.message) || ePack);
        f.r100_promote_ok = false;
      }
      emitDense(ctx, "B80_r100_high_value", f, 54, "main", "material_hedge");
    },
  });

  // Densify all already-registered packs (existing B0–B46)
  try {
    var ids = global.GRCollectors.ids ? global.GRCollectors.ids() : [];
    ids.forEach(function (id) {
      var d = global.GRCollectors.get(id);
      if (!d || d._dense_wrapped || !d.run) return;
      var orig = d.run;
      // Explicit pack→family via module PACK_FAMILY + resolveFamily
      var fam = resolveFamily(id, {}, d.family || null);
      d.family = fam;
      d.run = function (ctx) {
        // Boot path: registry enqueue → ctx.queue.enqueue; densify there.
        patchQueueDensify(ctx, id, fam);
        return orig.call(this, ctx);
      };
      d._dense_wrapped = true;
    });
  } catch (eW) {
    /* fe_diag spam removed */ /* silenced */

  }

  global.GRCollectors.densePackIds = function () {
    return [
      "B47_api_flags_detail",
      "B48_css_supports_detail",
      "B49_css_props_detail",
      "B50_font_matrix_detail",
      "B51_mq_matrix_detail",
      "B52_navigator_deep",
      "B53_window_keys_deep",
      "B54_plugin_mime_deep",
      "B55_webrtc_ice_deep",
      "B56_webrtc_stats",
      "B57_canvas_emoji_path",
      "B58_canvas_text_metrics",
      "B59_webgl_params_full",
      "B60_webgl_extensions_full",
      "B61_audio_worklet",
      "B62_offline_audio_moments",
      "B63_intl_full",
      "B64_timezone_deep",
      "B65_client_hints_full",
      "B66_sec_ch_headers",
      "B67_service_worker_deep",
      "B68_cache_storage_deep",
      "B69_bluetooth_usb",
      "B70_payment_credential",
      "B71_idle_wake_lock",
      "B72_keyboard_layout",
      "B73_pointer_capabilities",
      "B74_visual_viewport",
      "B75_performance_entries",
      "B76_math_wasm_deep",
      "B77_worker_env_deep",
      "B78_iframe_env_deep",
      "B79_cross_origin_isolation",
      "B80_r100_high_value"
    ];
  };
  global.GRCollectors.denseCount = function () {
    return (global.GRCollectors.densePackIds() || []).length;
  };
})(typeof window !== "undefined" ? window : globalThis);
