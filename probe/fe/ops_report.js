/**
 * Silent client ops reporter — never touches console for its own traffic.
 * POSTs compact health/error/console events to first-party /v1/ops/client_event.
 * Always includes product_version for version-scoped error triage.
 */
(function (global) {
  "use strict";
  var ENDPOINT = "/v1/ops/client_event";
  // Open collection: low site traffic — allow richer multipath/upload diagnostics.
  var MAX_PER_MIN = 80;
  var sent = [];
  var halted = false;
  var seenConsoleKeys = Object.create(null);
  var CONSOLE_DEDUP_MS = 30000;

  /**
   * Client error/ops upload enable (default on).
   * Disable via: __GR_BOOT__.ops_client_events=false | __GR_OPS_CLIENT_EVENTS__=0 | GROps.setEnabled(false)
   * Server may also reject when admin setting ops_client_events_enabled=0.
   */
  function clientEventsEnabled() {
    try {
      if (global.__GR_OPS_CLIENT_EVENTS__ === 0 || global.__GR_OPS_CLIENT_EVENTS__ === false)
        return false;
      if (global.__GR_OPS_CLIENT_EVENTS__ === 1 || global.__GR_OPS_CLIENT_EVENTS__ === true)
        return true;
      var boot = global.__GR_BOOT__ || {};
      if (boot.ops_client_events === false || boot.ops_client_events === 0) return false;
      if (boot.opsClientEvents === false || boot.opsClientEvents === 0) return false;
      if (String(boot.ops_client_events || "").toLowerCase() === "off") return false;
    } catch (e) {}
    return !halted;
  }

  function apiBase() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var b = String(boot.apiBase || boot.first_party_path || global.__GR_API_BASE__ || "/g5");
      return b.replace(/\/$/, "") || "/g5";
    } catch (e) {
      return "/g5";
    }
  }

  function prune() {
    var now = Date.now();
    sent = sent.filter(function (t) {
      return now - t < 60000;
    });
  }

  function uaHash() {
    try {
      var ua = String((global.navigator && navigator.userAgent) || "");
      var h = 2166136261;
      for (var i = 0; i < ua.length; i++) {
        h ^= ua.charCodeAt(i);
        h = Math.imul(h, 16777619);
      }
      return ("00000000" + (h >>> 0).toString(16)).slice(-8);
    } catch (e) {
      return "";
    }
  }

  /**
   * Engine family without InstallTrigger (deprecated — typeof alone warns in Firefox).
   * Capability-first: mozInnerScreenX / -moz- CSS; then real Gecko UA (not "like Gecko").
   */
  function isGeckoEngine(ua) {
    ua = String(ua || "");
    try {
      if (typeof global.mozInnerScreenX === "number") return true;
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

  function engineFamily() {
    try {
      var ua = String((global.navigator && navigator.userAgent) || "");
      if (isGeckoEngine(ua)) return "gecko";
      if (/AppleWebKit\/605/.test(ua) && /Safari\//.test(ua) && !/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua))
        return "webkit";
      if (/Chrome\/|Chromium\/|Edg\/|OPR\//.test(ua) || /CriOS\//.test(ua)) return "blink";
      if (/AppleWebKit\//.test(ua) && /Safari\//.test(ua) && !/Chrome\/|Chromium\/|CriOS\//.test(ua))
        return "webkit";
      return "unknown";
    } catch (e2) {
      return "unknown";
    }
  }

  function productVersion() {
    try {
      var boot = global.__GR_BOOT__ || {};
      return String(
        boot.version ||
          boot.product_version ||
          global.__GR_PRODUCT_VERSION__ ||
          ""
      );
    } catch (e) {
      return "";
    }
  }

  /**
   * Resolve visitor_terminal_id from all known FE surfaces so upload error/warn
   * rows always join to VT (retry outcome verification).
   */
  function resolveVisitorTerminalId() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var candidates = [
        global.__GR_VTID__,
        boot.visitor_terminal_id,
        boot.visitorTerminalId,
        boot.vt,
        global.__GR_VISITOR_TERMINAL_ID__,
      ];
      try {
        if (global.GRStorage && typeof GRStorage.getVt === "function") {
          candidates.push(GRStorage.getVt());
        }
      } catch (eS) {}
      try {
        if (typeof localStorage !== "undefined") {
          candidates.push(localStorage.getItem("gr_visitor_terminal_v1"));
          candidates.push(localStorage.getItem("_g5_vt"));
        }
      } catch (eL) {}
      for (var i = 0; i < candidates.length; i++) {
        var v = candidates[i];
        if (v != null && String(v).trim()) {
          var s = String(v).trim().slice(0, 96);
          try {
            global.__GR_VTID__ = s;
          } catch (eSet) {}
          return s;
        }
      }
    } catch (eR) {}
    return "";
  }

  function resolveSessionId() {
    try {
      return String(
        global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.session_id) ||
          ""
      ).slice(0, 96);
    } catch (e) {
      return "";
    }
  }

  /**
   * High-volume lifecycle/noise codes — sample to keep ops usable.
   * Real faults (upload_5xx, hard_load_fail, open_fail, version_heal) stay 1.0.
   */
  var SAMPLE_RATES = {
    // Lifecycle remint is expected on navigation; keep tiny sample (info severity).
    cycle_remint: 0.02,
    vt_mint: 0.04,
    fe_diag: 0.02,
    hard_sla_retry: 0.1,
    need_hard_anchor: 0.1,
    privacy_guard: 0.02,
    multipath_done: 0.05,
    // Network fails are environmental (CF challenge / tab kill) — sample harder.
    upload_network: 0.2,
    upload_upstream_closed: 0.25,
    rpa_quiet: 0.1,
    upload_soft_exhausted: 0.2,
    upload_deepen_exhausted: 0.25,
    upload_hard_exhausted: 0.35,
    fe_pack_reload: 0.03,
    wave2_defer_b10: 0.08,
    b10_content_gate: 0.15,
    upload_4xx: 0.15,
    wave2_empty: 0.12,
    // Self-heal Gap Table (T/C/B) — sample; start/stop always useful once.
    gap_t_retry: 0.2,
    gap_c_retry: 0.35,
    gap_c_timeout: 0.6,
    gap_b_apply: 0.2,
    gap_satisfied: 0.15,
    gap_supervisor_start: 1,
    gap_supervisor_stop: 1,
    cool_without_silicon: 0.2,
    // Seal/WASM client failures are not HTTP 5xx — sample + hard cap (crawler storms).
    upload_seal_fail: 0.35,
    seal_wasm_fail: 0.25,
    seal_wasm_unsupported: 1,
    // Recovery / outcome always kept — verify retry eventually landed.
    upload_recovered: 1,
    upload_session_summary: 1,
    b10x_must_land_scheduled: 0.15,
  };
  var sampleSeen = Object.create(null);
  var codeCaps = Object.create(null);
  var CODE_CAPS = {
    cycle_remint: 1,
    vt_mint: 2,
    upload_4xx: 6,
    rpa_quiet: 3,
    upload_soft_exhausted: 4,
    upload_deepen_exhausted: 4,
    upload_hard_exhausted: 4,
    fe_pack_reload: 2,
    wave2_defer_b10: 2,
    b10_content_gate: 3,
    gap_t_retry: 8,
    gap_c_retry: 10,
    gap_c_timeout: 6,
    gap_b_apply: 8,
    gap_satisfied: 6,
    gap_supervisor_start: 2,
    gap_supervisor_stop: 2,
    fe_console_error: 5,
    upload_network: 8,
    upload_upstream_closed: 8,
    wave2_empty: 2,
    upload_recovered: 12,
    // Crawlers/fake kernels retry seal dozens of times — keep first few only.
    upload_seal_fail: 4,
    seal_wasm_fail: 3,
    seal_wasm_unsupported: 2,
    upload_5xx: 12,
  };

  function shouldSample(code) {
    var c = String(code || "");
    var cap = CODE_CAPS[c];
    if (cap != null) {
      codeCaps[c] = (codeCaps[c] || 0) + 1;
      if (codeCaps[c] > cap) return false;
    }
    var rate = SAMPLE_RATES[c];
    if (rate == null || rate >= 1) return true;
    // Always keep first event per code per page for triage, then sample.
    if (!sampleSeen[c]) {
      sampleSeen[c] = 1;
      return true;
    }
    try {
      return Math.random() < rate;
    } catch (eS) {
      return true;
    }
  }

  function report(code, stage, detail, severity) {
    if (!clientEventsEnabled() || !code) return;
    // Drop bogus empty fe_diag spam (was console.warn replaced with report("fe_diag")).
    if (String(code) === "fe_diag") {
      var d0 = detail && typeof detail === "object" ? detail : {};
      if (!d0.msg && !d0.err && !d0.detail && stage === "boot") return;
    }
    if (!shouldSample(code)) return;
    prune();
    if (sent.length >= MAX_PER_MIN) return;
    sent.push(Date.now());
    var boot = global.__GR_BOOT__ || {};
    var rate = SAMPLE_RATES[String(code)] != null ? SAMPLE_RATES[String(code)] : 1;
    var vt = resolveVisitorTerminalId();
    var sid = resolveSessionId();
    var body = {
      code: String(code).slice(0, 64),
      stage: String(stage || "client").slice(0, 32),
      severity: severity || "error",
      ts_ms: Date.now(),
      site_id: boot.site_id || boot.siteId || global.__GR_SITE_ID__ || "",
      visitor_terminal_id: vt,
      session_id: sid,
      product_version: productVersion(),
      inject_path: boot.injectPath || boot.inject_path || "",
      engine_family: engineFamily(),
      ua_hash: uaHash(),
      detail: detail && typeof detail === "object" ? detail : {},
      sample_rate: rate,
    };
    // Always mirror version + vtid/session inside detail for join/triage.
    try {
      if (body.detail && typeof body.detail === "object") {
        if (!body.detail.product_version) body.detail.product_version = body.product_version;
        if (!body.detail.visitor_terminal_id && vt) body.detail.visitor_terminal_id = vt;
        if (!body.detail.session_id && sid) body.detail.session_id = sid;
      }
    } catch (eD) {}
    var url = apiBase() + ENDPOINT;
    try {
      var blob = new Blob([JSON.stringify(body)], { type: "text/plain" });
      if (global.navigator && typeof navigator.sendBeacon === "function") {
        if (navigator.sendBeacon(url, blob)) return;
      }
    } catch (eB) {}
    try {
      if (typeof fetch === "function") {
        fetch(url, {
          method: "POST",
          credentials: "same-origin",
          keepalive: true,
          headers: { "Content-Type": "text/plain" },
          body: JSON.stringify(body),
        }).catch(function () {});
      }
    } catch (eF) {}
  }

  /** Gated debug log — only when server/open enabled debug. Never default-on. */
  function dlog() {
    try {
      if (!(global.__GR_DEBUG__ || (global.__GR_BOOT__ && global.__GR_BOOT__.debug))) return;
      if (arguments && arguments.length) {
        report("debug_trace", "debug", { n: arguments.length }, "info");
      }
    } catch (e) {}
  }

  /** Browser / extension noise we must not flood into ops. */
  function isBrowserNoise(msg) {
    msg = String(msg || "");
    if (!msg) return true;
    return /InstallTrigger is deprecated|Fingerprinting Protection is altering|WebGPU is experimental|Failed to create WebGPU Context Provider|gpuweb\/wiki\/Implementation|moz-extension:|chrome-extension:|Download the React DevTools|\[HMR\]|webpack|favicon\.ico|net::ERR_BLOCKED_BY_CLIENT|ResizeObserver loop|Script error\.$|Script terminated by timeout|\[NEW\] Explain Console|无法访问Iframe|cross-origin frame|Blocked a frame with origin|Falling back to browser navigation|Failed to fetch RSC payload|SecurityError.*frame|from accessing a cross-origin/i.test(
      msg
    );
  }

  function argsToMsg(args) {
    try {
      var parts = [];
      for (var i = 0; i < (args ? args.length : 0) && i < 6; i++) {
        var a = args[i];
        if (a == null) {
          parts.push(String(a));
        } else if (typeof a === "string") {
          parts.push(a);
        } else if (a && a.message) {
          parts.push(String(a.message));
        } else if (a && a.stack) {
          parts.push(String(a.stack).slice(0, 200));
        } else {
          try {
            parts.push(JSON.stringify(a).slice(0, 160));
          } catch (eJ) {
            parts.push(String(a));
          }
        }
      }
      return parts.join(" ").slice(0, 360);
    } catch (e) {
      return "";
    }
  }

  function consoleDedupOk(key) {
    var now = Date.now();
    var prev = seenConsoleKeys[key] || 0;
    if (now - prev < CONSOLE_DEDUP_MS) return false;
    seenConsoleKeys[key] = now;
    // Bound map size
    var keys = Object.keys(seenConsoleKeys);
    if (keys.length > 80) {
      for (var i = 0; i < 40; i++) delete seenConsoleKeys[keys[i]];
    }
    return true;
  }

  function reportConsole(level, args) {
    try {
      var msg = argsToMsg(args);
      if (!msg || isBrowserNoise(msg)) return;
      // Prefer GR / probe-relevant lines; still take bare Error / TypeError from our pages.
      var relevant =
        /gr|GR|WebGL|WEBGL|probe|registry|upload|ingest|B\d+_|\bR\d+_spot|pack_loader|context lost|INVALID_ENUM|out of memory|TypeError|ReferenceError|SyntaxError|Failed to fetch|NetworkError/i.test(
          msg
        );
      if (!relevant && level === "warn") return;
      if (!relevant && level === "error") {
        // Still capture generic errors when on our SDK path (filename-less console.error).
        if (!/error|fail|exception|reject/i.test(msg)) return;
      }
      var code = level === "warn" ? "fe_console_warn" : "fe_console_error";
      var sev = level === "warn" ? "warn" : "error";
      var dkey = code + ":" + msg.slice(0, 80);
      if (!consoleDedupOk(dkey)) return;
      report(
        code,
        "console",
        {
          level: level,
          msg: msg.slice(0, 320),
          href: (function () {
            try {
              return String((global.location && location.pathname) || "").slice(0, 120);
            } catch (eH) {
              return "";
            }
          })(),
        },
        sev
      );
    } catch (eC) {}
  }

  // Hook runtime errors + console.warn/error (console intercept is the only way
  // to surface WebGL/browser warn lines that never fire window.onerror).
  try {
    if (!global.__GR_OPS_HOOKED__) {
      global.__GR_OPS_HOOKED__ = true;
      var lastGlLost = 0;
      global.addEventListener(
        "webglcontextlost",
        function (ev) {
          var now = Date.now();
          if (now - lastGlLost < 5000) return;
          lastGlLost = now;
          report(
            "webgl_context_lost",
            "webgl",
            {
              status: "lost",
              target: ev && ev.target ? String(ev.target.tagName || "canvas") : "canvas",
            },
            "warn"
          );
        },
        true
      );
      global.addEventListener("error", function (ev) {
        try {
          var msg = String((ev && (ev.message || (ev.error && ev.error.message))) || "");
          if (!msg || isBrowserNoise(msg)) return;
          var src = String((ev && ev.filename) || "");
          var ours =
            /WebGL|WEBGL|Script error|out of memory|Allocation failed|TypeError|ReferenceError/i.test(
              msg
            ) ||
            (src && /gr|registry|collectors|pack_loader|upload_queue|ops_report/i.test(src));
          if (!ours) return;
          report(
            "fe_runtime_error",
            "error",
            {
              msg: msg.slice(0, 160),
              src: src.slice(0, 80),
              line: ev && ev.lineno,
              col: ev && ev.colno,
            },
            "error"
          );
        } catch (eE) {}
      });
      global.addEventListener("unhandledrejection", function (ev) {
        try {
          var reason = ev && ev.reason;
          var msg = String((reason && (reason.message || reason)) || reason || "");
          if (!msg || isBrowserNoise(msg)) return;
          if (/WebGL|upload|probe|GR|multipath|fetch|network|TypeError|reject/i.test(msg)) {
            report(
              "fe_unhandled_rejection",
              "error",
              { msg: msg.slice(0, 160) },
              "warn"
            );
          }
        } catch (eR) {}
      });

      // Wrap console.error / console.warn (preserve original; never throw).
      try {
        var c = global.console;
        if (c && !global.__GR_OPS_CONSOLE__) {
          global.__GR_OPS_CONSOLE__ = true;
          var origError = typeof c.error === "function" ? c.error.bind(c) : null;
          var origWarn = typeof c.warn === "function" ? c.warn.bind(c) : null;
          if (origError) {
            c.error = function () {
              try {
                reportConsole("error", arguments);
              } catch (e1) {}
              try {
                return origError.apply(c, arguments);
              } catch (e2) {}
            };
          }
          if (origWarn) {
            c.warn = function () {
              try {
                reportConsole("warn", arguments);
              } catch (e3) {}
              try {
                return origWarn.apply(c, arguments);
              } catch (e4) {}
            };
          }
        }
      } catch (eHookC) {}
    }
  } catch (eHook) {}

  global.GROps = {
    report: report,
    dlog: dlog,
    engineFamily: engineFamily,
    productVersion: productVersion,
    wave2Empty: function (extra) {
      // Dedupe: hard module race can fire 3× wave2Empty per page.
      try {
        var t = Date.now();
        if (global.__GR_LAST_WAVE2_EMPTY_MS__ && t - global.__GR_LAST_WAVE2_EMPTY_MS__ < 8000) {
          return;
        }
        global.__GR_LAST_WAVE2_EMPTY_MS__ = t;
      } catch (eW) {}
      // Expected pack-schedule lifecycle (hard not ready yet / benign empty).
      report("wave2_empty", "kick", extra || {}, "info");
    },
    hardLoadFail: function (pack, err) {
      report(
        "hard_load_fail",
        "load_pack",
        { pack: String(pack || ""), err: String(err || "").slice(0, 240) },
        "error"
      );
    },
    uploadHttp: function (status, batch, extra) {
      var s = Number(status) || 0;
      extra = extra || {};
      var bid = String(batch || "").slice(0, 48);
      var errStr = String(extra.err || extra.message || extra.body_snip || "");
      var cls = String(extra.net_class || errStr || "").toLowerCase();
      // Never label HTTP 2xx as 4xx (webkit false positive was http:200 + upload_4xx).
      if (s > 0 && s < 400) {
        report(
          "upload_biz_reject",
          "upload",
          Object.assign({ http: s, batch: bid }, extra),
          "warn"
        );
        return;
      }
      // --- Seal / WASM client failures (NOT HTTP 5xx) ---
      // Prod v148: ~half of "upload_5xx" were seal_wasm_required / seal_failed with http:0
      // on crawler/fake-kernel sessions (gateway_only). Classify for ops integrity.
      var sealFlag =
        extra.seal_failed === true ||
        extra.seal_required === true ||
        extra.seal_failed === 1 ||
        String(extra.code || "") === "seal_failed" ||
        String(extra.code || "") === "sealed_required";
      var sealMsg =
        /seal_wasm|seal_failed|seal_module|seal_grant|seal_timeout|wasm_|webassembly/i.test(
          errStr
        ) || /seal_wasm|seal_failed|wasm_/i.test(cls);
      if (sealFlag || sealMsg) {
        var unsup = /wasm_unsupported|webassembly is not defined|no webassembly/i.test(
          errStr + " " + cls
        );
        var wasmLoad =
          unsup ||
          /seal_wasm_required|seal_wasm_load|wasm_http|wasm_not_binary|wasm_no_exports|instantiateStreaming|reached end while decod/i.test(
            errStr
          );
        var codeSeal = unsup
          ? "seal_wasm_unsupported"
          : wasmLoad
            ? "seal_wasm_fail"
            : "upload_seal_fail";
        // Crawler/incomplete engines often retry every batch — warn not error flood.
        var sevSeal = unsup || extra.final === true ? "error" : "warn";
        extra.net_class = extra.net_class || (wasmLoad ? "seal_wasm" : "seal_client");
        extra.seal_failed = true;
        report(
          codeSeal,
          "upload",
          Object.assign(
            {
              http: s,
              batch: bid,
              visitor_terminal_id: resolveVisitorTerminalId(),
              session_id: resolveSessionId(),
            },
            extra
          ),
          sevSeal
        );
        return;
      }
      var code = s >= 500 || s === 0 ? "upload_5xx" : "upload_4xx";
      var sev = "error";
      // Cloudflare / edge challenge (I'm Under Attack, JS challenge, 403 HTML).
      var isCf =
        s === 403 ||
        s === 503 ||
        /challenge|cf-mitigated|access denied|just a moment|attention required|under attack|cf-ray|cloudflare/i.test(
          cls
        );
      if (isCf) {
        code = s === 0 ? "upload_network" : "upload_4xx";
        sev = "warn";
        extra.net_class = extra.net_class || "edge_challenge";
        extra.cf_challenge = true;
      } else if (s === 0 && extra.network) {
        // http:0 + network = browser never got HTTP status (not API 4xx/5xx).
        // Subtypes: offline | client_fetch_fail | edge_challenge | abort | backgrounded.
        code = "upload_network";
        if (extra.online === false || cls === "offline") {
          extra.net_class = "offline";
        } else if (!extra.net_class || extra.net_class === "fetch_fail") {
          extra.net_class = extra.net_class || "client_fetch_fail";
        }
        extra.transport = extra.transport || "no_http_response";
        // Mid-retry network blips → warn; final deepen exhaust also warn (not API outage).
        var willRetry = extra.will_retry !== false && extra.final !== true;
        if (
          extra.backgrounded ||
          extra.timeout_abort ||
          willRetry ||
          /abort|offline|client_fetch/i.test(cls + " " + (extra.net_class || ""))
        ) {
          sev = "warn";
        } else {
          // Final network fail without HTTP → still warn (domain/CDN/client), not 5xx error.
          sev = "warn";
        }
      } else if (s === 0 && !extra.network) {
        // Ambiguous http:0 without network flag — prefer network/warn over false 5xx.
        code = "upload_network";
        sev = "warn";
        extra.net_class = extra.net_class || "http0_unclassified";
        extra.transport = extra.transport || "no_http_response";
      } else if (
        // Edge/proxy dropped the TCP stream mid-response: often surfaces as
        // HTTP 500 + "connection closed" (nginx/CF/browser), not app 5xx logic.
        // Prod v148: large share of human thin_surface upload_5xx was this pattern.
        /connection closed|broken pipe|econnreset|err_connection|connection reset|upstream prematurely|socket hang up/i.test(
          errStr + " " + cls
        )
      ) {
        code = "upload_upstream_closed";
        sev = extra.final === true ? "error" : "warn";
        extra.net_class = extra.net_class || "upstream_closed";
        extra.network = true;
      } else if (s >= 500) {
        code = "upload_5xx";
        sev = "error";
      }
      report(
        code,
        "upload",
        Object.assign(
          {
            http: s,
            batch: bid,
            visitor_terminal_id: resolveVisitorTerminalId(),
            session_id: resolveSessionId(),
          },
          extra
        ),
        sev
      );
    },
    /**
     * Report that a batch eventually uploaded after prior network/HTTP failure.
     * Always includes visitor_terminal_id for retry-outcome verification.
     */
    uploadRecovered: function (batch, extra) {
      extra = extra || {};
      report(
        "upload_recovered",
        "upload",
        Object.assign(
          {
            batch: String(batch || "").slice(0, 48),
            visitor_terminal_id: resolveVisitorTerminalId(),
            session_id: resolveSessionId(),
            recovered: true,
          },
          extra
        ),
        "info"
      );
    },
    resolveVisitorTerminalId: resolveVisitorTerminalId,
    resolveSessionId: resolveSessionId,
    cycleRemint: function (reason, extra) {
      var r = String(reason || "");
      // Normal navigation mint is info; only version mismatch / supersede stay warn.
      var sev =
        r === "fresh_cycle" || r === "session_storage_page_budget" || !r ? "info" : "warn";
      report(
        "cycle_remint",
        "lifecycle",
        Object.assign({ reason: r }, extra || {}),
        sev
      );
    },
    openFail: function (extra) {
      report("open_fail", "open", extra || {}, "error");
    },
    retryBudget: function (extra) {
      report("retry_budget", "retry", extra || {}, "warn");
    },
    versionUpgrade: function (selfV, serverV, action) {
      report(
        "version_upgrade",
        "self_heal",
        { self: String(selfV || ""), server: String(serverV || ""), action: String(action || "") },
        "info"
      );
    },
    /** Canonical version self-heal event (queryable: version_heal). */
    versionHeal: function (selfV, serverV, action, extra) {
      report(
        "version_heal",
        "self_heal",
        Object.assign(
          {
            self: String(selfV || ""),
            server: String(serverV || ""),
            action: String(action || ""),
          },
          extra || {}
        ),
        "info"
      );
    },
    /** Canonical vt mint with reason (queryable: vt_mint + detail.reason / vt_mint_reason). */
    vtMint: function (reason, vt, extra) {
      report(
        "vt_mint",
        "lifecycle",
        Object.assign(
          {
            reason: String(reason || "unknown"),
            vt_mint_reason: String(reason || "unknown"),
            vt: String(vt || "").slice(0, 64),
          },
          extra || {}
        ),
        "info"
      );
    },
    /** Explicit FE error/warn (call from probe code); always version-keyed. */
    feError: function (code, detail, severity) {
      report(String(code || "fe_error").slice(0, 64), "client", detail || {}, severity || "error");
    },
    /** Enable/disable client ops/error upload (panel / inject config). */
    setEnabled: function (on) {
      try {
        global.__GR_OPS_CLIENT_EVENTS__ = on ? 1 : 0;
        halted = !on;
      } catch (e) {}
    },
    isEnabled: function () {
      return clientEventsEnabled();
    },
    /** Halt uploads for this page (lifecycle complete / policy). */
    halt: function () {
      halted = true;
    },
  };
})(typeof window !== "undefined" ? window : globalThis);
