/**
 * Continuous RPA event monitor — bind once per surface (main / iframe / worker).
 * Does NOT wait for route_plan. Each surface owns its events; never re-tag another source.
 *
 * Usage (browser main/iframe):
 *   GRRpaMonitor.bind({ source, session_id, page_id, page_url, queue|post, inject_path })
 * Usage (worker):
 *   GRRpaMonitor.bindWorker({ source, postMessage })
 */
(function (global) {
  "use strict";

  /**
   * Idle threshold before idle flush (ms).
   * iss/50 R2: MUST match server `RPA_IDLE_ANALYZE_MS` (gr-core return_gate.rs = 30000).
   * Reason codes: `idle_30s` (preferred); `idle_45s` still accepted server-side for old clients.
   */
  var RPA_IDLE_MS = 30000;
  /** Total event buffer soft cap (moves + actions). */
  var EVENT_CAP = 80;
  /**
   * iss/50 R1 dual-track: high-value action events are protected from a pure move FIFO.
   * Moves fill MOVE_CAP; actions fill ACTION_CAP; when full, drop oldest **move** first.
   */
  var MOVE_CAP = 48;
  var ACTION_CAP = 32;
  /** Continuous flush interval — was 2s (21× B11/session). 8s still covers maximize RPA. */
  var CONT_FLUSH_MS = 8000;
  /** Cap continuous B11 uploads per surface per page lifetime. */
  var CONT_FLUSH_MAX = 8;

  function isMoveKind(kind) {
    var k = String(kind || "");
    return (
      k === "pointermove" ||
      k === "mousemove" ||
      k === "touchmove" ||
      k === "pointerrawupdate"
    );
  }

  function isActionKind(kind) {
    return !isMoveKind(kind);
  }

  /** Drop oldest move-class event; return true if dropped. */
  function dropOldestMove(arr) {
    for (var i = 0; i < arr.length; i++) {
      if (isMoveKind(arr[i].kind || arr[i].type)) {
        arr.splice(i, 1);
        return true;
      }
    }
    return false;
  }

  function countKind(arr, pred) {
    var n = 0;
    for (var i = 0; i < arr.length; i++) {
      if (pred(arr[i].kind || arr[i].type)) n++;
    }
    return n;
  }

  /**
   * Dual-track retention (iss/50 R1): never let move flood alone evict all actions.
   * When over cap: prefer dropping oldest move; only drop action if no moves remain.
   */
  function dualTrackAdmit(arr, kind) {
    var move = isMoveKind(kind);
    // Soft per-track caps
    if (move) {
      while (countKind(arr, isMoveKind) >= MOVE_CAP) {
        if (!dropOldestMove(arr)) break;
      }
    } else {
      while (countKind(arr, isActionKind) >= ACTION_CAP && arr.length) {
        // Drop oldest **action** only when action track is full (not moves).
        var dropped = false;
        for (var i = 0; i < arr.length; i++) {
          if (isActionKind(arr[i].kind || arr[i].type)) {
            arr.splice(i, 1);
            dropped = true;
            break;
          }
        }
        if (!dropped) break;
      }
    }
    // Global cap: protect actions by dropping moves first
    while (arr.length >= EVENT_CAP) {
      if (dropOldestMove(arr)) continue;
      arr.shift();
    }
  }

  function pageUrlDefault() {
    try {
      return String((typeof location !== "undefined" && location.href) || "");
    } catch (e) {
      return "";
    }
  }

  /** Stable path without query/hash for page scope (privacy). */
  function pagePathKey(href) {
    try {
      var u = new URL(href || pageUrlDefault(), "https://local.invalid");
      return String(u.origin || "") + String(u.pathname || "/");
    } catch (e) {
      return String(href || "").split("?")[0].split("#")[0];
    }
  }

  /** Per-tab/window nonce (sessionStorage) — page_id = f(url_path, tab). */
  function tabWindowId(win) {
    win = win || global;
    try {
      var ss = win.sessionStorage;
      if (!ss) return "t0";
      var k = "__gr_tab_win_id__";
      var v = ss.getItem(k);
      if (!v) {
        v =
          "t" +
          Math.random().toString(36).slice(2, 10) +
          Date.now().toString(36).slice(-6);
        ss.setItem(k, v);
      }
      return v;
    } catch (e) {
      return "t0";
    }
  }

  function simpleHash16(s) {
    var h = 2166136261;
    s = String(s || "");
    for (var i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return ("00000000" + (h >>> 0).toString(16)).slice(-8);
  }

  /** Page-scoped RPA key: unique per URL path + browser tab/window. */
  function makePageId(href, win) {
    var path = pagePathKey(href);
    var tab = tabWindowId(win);
    return "p_" + simpleHash16(path + "|" + tab) + simpleHash16(tab + "|v1").slice(0, 8);
  }

  /** Redact raw keyboard chars (iss/47) — keep only class for dwell stats. */
  function keyClass(key, code) {
    var k = String(key || "");
    var c = String(code || "");
    if (!k && !c) return "";
    if (k.length === 1) {
      if (/[0-9]/.test(k)) return "digit";
      if (/[a-zA-Z]/.test(k)) return "letter";
      return "char";
    }
    if (/Enter|Tab|Escape|Backspace|Delete|Space|Arrow/.test(c) || /Enter|Tab|Escape/.test(k)) {
      return "nav";
    }
    if (/Shift|Control|Alt|Meta/.test(c) || /Shift|Control|Alt|Meta/.test(k)) {
      return "mod";
    }
    return "other";
  }

  /**
   * Bind continuous RPA monitors on a Document (main or iframe).
   * opts:
   *  - source: "main" | "iframe:d1" | "sandbox_iframe:d1" | ...
   *  - session_id
   *  - page_id, page_url
   *  - inject_path
   *  - queue: { enqueue(row) }  OR  post: function(fields, reason) for parent bridge
   *  - doc: Document (default document)
   *  - win: Window (default window/global)
   *  - boundKey: optional unique key so multi-surfaces don't collide
   */
  function bind(opts) {
    opts = opts || {};
    var win = opts.win || global;
    var doc = opts.doc || (win && win.document) || (typeof document !== "undefined" ? document : null);
    var source = opts.source || "main";
    var boundKey = opts.boundKey || ("__GR_RPA_BOUND_" + source + "__");
    if (win[boundKey]) {
      return win[boundKey];
    }

    var pageUrl = opts.page_url || pageUrlDefault();
    var state = {
      source: source,
      events: [],
      lastEventMs: 0,
      lastUploadMs: 0,
      bound: true,
      session_id: opts.session_id || "",
      // Page-level: url_path + tab/window id (not session-wide)
      page_id: opts.page_id || makePageId(pageUrl, win),
      page_url: pageUrl,
      page_path: pagePathKey(pageUrl),
      tab_window_id: tabWindowId(win),
      inject_path: opts.inject_path || null,
      nextSeq: 1,
      ackedSeq: 0,
      inflightSeg: null,
    };
    try {
      win.__GR_PAGE_ID__ = state.page_id;
      win.__GR_TAB_WINDOW_ID__ = state.tab_window_id;
    } catch (ePid) {}

    state.bindAtMs = Date.now();
    state.sensitiveZeroMove = false;
    state.sensitiveActionSeen = false;

    function push(kind, ev) {
      dualTrackAdmit(state.events, kind);
      var xf = Number((ev && (ev.clientX != null ? ev.clientX : ev.x)) || 0);
      var yf = Number((ev && (ev.clientY != null ? ev.clientY : ev.y)) || 0);
      var row = {
        t: Date.now(),
        kind: kind,
        type: kind,
        source: source,
        event_seq: state.nextSeq++,
        // Privacy bucket for any residual upload; kinematics use _xf/_yf full precision (iss/50 D2)
        x: Math.round(xf / 4) * 4,
        y: Math.round(yf / 4) * 4,
        _xf: xf,
        _yf: yf,
        _track: isMoveKind(kind) ? "move" : "action",
      };
      // iss/47: do not upload raw key/code — keep class for dwell pairing
      if (ev && (ev.key != null || ev.code != null)) {
        row.key_class = keyClass(ev.key, ev.code);
      }
      if (ev && ev.deltaY != null) row.deltaY = Number(ev.deltaY) || 0;
      // Touch modality (no fingerprints of contact geometry beyond count)
      try {
        if (ev && ev.touches && ev.touches.length) {
          row.touch_count = ev.touches.length;
        } else if (ev && ev.changedTouches && ev.changedTouches.length) {
          row.touch_count = ev.changedTouches.length;
        } else if (ev && ev.pointerType) {
          row.pointer_type = String(ev.pointerType).slice(0, 16);
        }
      } catch (eT) {}
      // Coalesced pointer samples (synthetic / high-rate humanizers often lack these)
      try {
        if (ev && typeof ev.getCoalescedEvents === "function") {
          var ce = ev.getCoalescedEvents();
          row.coalesced_n = ce && ce.length ? ce.length : 0;
        }
      } catch (eC) {}
      state.events.push(row);
      state.lastEventMs = Date.now();
    }

    /** iss/21 T-BIO-1 + 001 RPA: derive input-physics summaries from event stream (claim-free). */
    function kinematicsSummary(events) {
      var out = {
        input_mouse_entropy: null,
        integer_coord_ratio: null,
        key_dwell_stddev: null,
        key_flight_cv: null,
        key_hold_mean_ms: null,
        pre_action_move_count: 0,
        behavior_type_diversity_n: 0,
        ttfi_ms: null,
        event_order_score: null,
        scroll_burst_n: 0,
        input_velocity_cv: null,
        input_jerk_cv: null,
        path_length_px: null,
        path_straightness: null,
        path_curvature_mean: null,
        click_interval_cv: null,
        overshoot_n: 0,
        fitts_residual_mean: null,
        window_risk_max: null,
        rpa_signal: "none",
        sensitive_action_zero_move: !!state.sensitiveZeroMove,
      };
      if (!events || !events.length) {
        out.rpa_signal = "none";
        return out;
      }
      var types = {};
      var moveKinds = { mousemove: 1, pointermove: 1, touchmove: 1, wheel: 1 };
      var actionKinds = { click: 1, pointerdown: 1, pointerup: 1, keydown: 1, touchstart: 1 };
      var coords = 0;
      var intCoords = 0;
      var cells = {};
      var preActionMoves = 0;
      var sawAction = false;
      var keyDownAt = {};
      var dwells = [];
      var firstInteractT = null;
      var pathLen = 0;
      var prevX = null;
      var prevY = null;
      var firstX = null;
      var firstY = null;
      var speeds = [];
      var accels = [];
      var jerks = [];
      var prevSp = null;
      var prevAcc = null;
      var prevT = null;
      var scrollBurst = 0;
      var scrollWindow = 0;
      var scrollWindowStart = 0;
      var clickGaps = [];
      var lastClickT = null;
      var flights = [];
      var lastKeyUpT = null;
      var curvSamples = [];
      var overshootN = 0;
      var fittsResiduals = [];
      var winRisks = [];
      // Expected humanish order score: moves before click; keydown before keyup
      var orderHits = 0;
      var orderTotal = 0;
      for (var i = 0; i < events.length; i++) {
        var e = events[i] || {};
        var k = String(e.kind || e.type || "");
        types[k] = 1;
        var t = Number(e.t) || 0;
        if (firstInteractT == null && (moveKinds[k] || actionKinds[k] || k === "scroll")) {
          firstInteractT = t;
        }
        if (!sawAction && moveKinds[k]) preActionMoves++;
        if (actionKinds[k]) {
          orderTotal++;
          if (preActionMoves > 0 || sawAction) orderHits++;
          sawAction = true;
        }
        if (k === "scroll" || k === "wheel") {
          if (!scrollWindowStart || t - scrollWindowStart > 400) {
            scrollWindowStart = t;
            scrollWindow = 1;
          } else {
            scrollWindow++;
            if (scrollWindow >= 4) scrollBurst++;
          }
        }
        if ((e._xf != null || e.x != null) && (moveKinds[k] || k === "click" || k === "pointerdown")) {
          coords++;
          // Full precision for integer/subpixel signal (D2: never use 4px-bucketed x/y)
          var x = Number(e._xf != null ? e._xf : e.x);
          var y = Number(e._yf != null ? e._yf : e.y);
          // Near-integer at 1px: humans often have subpixel; scripts often pure int
          if (isFinite(x) && isFinite(y)) {
            var nearInt =
              Math.abs(x - Math.round(x)) < 0.02 && Math.abs(y - Math.round(y)) < 0.02;
            if (nearInt) intCoords++;
          }
          var cx = Math.floor(x / 8);
          var cy = Math.floor(y / 8);
          cells[cx + "," + cy] = 1;
          if (firstX == null && isFinite(x) && isFinite(y)) {
            firstX = x;
            firstY = y;
          }
          if (prevX != null && prevY != null && isFinite(x) && isFinite(y)) {
            var dx = x - prevX;
            var dy = y - prevY;
            var dist = Math.sqrt(dx * dx + dy * dy);
            pathLen += dist;
            if (prevT != null && t > prevT) {
              var dt = Math.max(1, t - prevT);
              var sp = dist / dt;
              if (isFinite(sp) && sp >= 0) {
                speeds.push(sp);
                if (prevSp != null) {
                  var acc = (sp - prevSp) / dt;
                  accels.push(acc);
                  if (prevAcc != null) {
                    jerks.push((acc - prevAcc) / dt);
                  }
                  prevAcc = acc;
                }
                prevSp = sp;
              }
              // curvature proxy: angle change between segments
              if (i >= 2) {
                var ePrev = events[i - 1] || {};
                var ePrev2 = events[i - 2] || {};
                var x1 = Number(ePrev2._xf != null ? ePrev2._xf : ePrev2.x);
                var y1 = Number(ePrev2._yf != null ? ePrev2._yf : ePrev2.y);
                var x2 = Number(ePrev._xf != null ? ePrev._xf : ePrev.x);
                var y2 = Number(ePrev._yf != null ? ePrev._yf : ePrev.y);
                if (isFinite(x1) && isFinite(x2) && isFinite(y1) && isFinite(y2)) {
                  var a1 = Math.atan2(y2 - y1, x2 - x1);
                  var a2 = Math.atan2(y - y2, x - x2);
                  var dang = Math.abs(a2 - a1);
                  if (dang > Math.PI) dang = 2 * Math.PI - dang;
                  if (isFinite(dang)) curvSamples.push(dang);
                }
              }
            }
          }
          prevX = x;
          prevY = y;
          prevT = t;
        }
        if ((k === "click" || k === "pointerdown") && t > 0) {
          if (lastClickT != null && t > lastClickT) clickGaps.push(t - lastClickT);
          // overshoot: approach then reverse before click (simple heuristic)
          if (speeds.length >= 3 && prevSp != null && prevSp < speeds[speeds.length - 2] * 0.5) {
            overshootN++;
          }
          // Fitts residual: T vs a+b*log2(D/W+1); W≈24px target heuristic
          if (firstX != null && prevX != null && firstInteractT != null) {
            var D = Math.sqrt((prevX - firstX) * (prevX - firstX) + (prevY - firstY) * (prevY - firstY));
            var W = 24;
            var Tobs = Math.max(1, t - (firstInteractT || t));
            var Tpred = 80 + 120 * (Math.log(D / W + 1) / Math.LN2);
            if (isFinite(Tobs) && isFinite(Tpred)) {
              fittsResiduals.push((Tobs - Tpred) / Math.max(1, Tpred));
            }
          }
          lastClickT = t;
        }
        if (k === "keydown" && e.t != null) {
          // Pair by key_class (not constant "k") — iss/50 D2
          var kdn = Number(e.t) || 0;
          keyDownAt[String(e.key_class || e.key || e.code || "k")] = kdn;
          if (lastKeyUpT != null && kdn > lastKeyUpT) {
            var fl = kdn - lastKeyUpT;
            if (fl > 0 && fl < 8000) flights.push(fl);
          }
        }
        if (k === "keyup" && e.t != null) {
          var kkey = String(e.key_class || e.key || e.code || "k");
          var kd = keyDownAt[kkey];
          orderTotal++;
          if (kd) {
            orderHits++;
            var d = (Number(e.t) || 0) - kd;
            if (d > 0 && d < 5000) dwells.push(d);
            delete keyDownAt[kkey];
          }
          lastKeyUpT = Number(e.t) || 0;
        }
      }
      // 5s sliding window risk (metronomic speed / zero jerk)
      if (events.length >= 8 && firstInteractT != null) {
        var winMs = 5000;
        var stepMs = 2000;
        var tEnd = Number(events[events.length - 1].t) || firstInteractT;
        for (var wt = firstInteractT; wt < tEnd; wt += stepMs) {
          var wspeeds = [];
          for (var wi = 0; wi < events.length; wi++) {
            var we = events[wi] || {};
            var wtt = Number(we.t) || 0;
            if (wtt < wt || wtt >= wt + winMs) continue;
            var wk = String(we.kind || we.type || "");
            if (moveKinds[wk] && we._xf != null) wspeeds.push(1);
          }
          if (wspeeds.length >= 6) {
            // dense window with low diversity → risk
            var risk = wspeeds.length > 40 ? 0.7 : 0.35;
            winRisks.push(risk);
          }
        }
      }
      out.pre_action_move_count = preActionMoves;
      out.behavior_type_diversity_n = Object.keys(types).length;
      out.scroll_burst_n = scrollBurst;
      out.path_length_px = pathLen > 0 ? Math.round(pathLen) : null;
      // Device modality mix (desktop mouse vs mobile touch) — industry bot/human split
      var touchN = 0;
      var mouseN = 0;
      var maxTouches = 0;
      for (var it = 0; it < events.length; it++) {
        var ek = String((events[it] && (events[it].kind || events[it].type)) || "");
        if (ek.indexOf("touch") === 0) {
          touchN++;
          var tc = events[it].touch_count != null ? Number(events[it].touch_count) : 1;
          if (tc > maxTouches) maxTouches = tc;
        }
        if (ek === "mousemove" || ek === "pointermove" || ek === "click") mouseN++;
      }
      out.touch_event_n = touchN;
      out.mouse_like_event_n = mouseN;
      out.touch_event_ratio =
        events.length > 0 ? Math.round((touchN / events.length) * 1000) / 1000 : null;
      out.max_concurrent_touches_seen = maxTouches > 0 ? maxTouches : null;
      out.input_modality =
        touchN > 0 && mouseN === 0 ? "touch" : touchN > 0 && mouseN > 0 ? "hybrid" : "pointer";
      if (state.bindAtMs && firstInteractT != null) {
        out.ttfi_ms = Math.max(0, firstInteractT - state.bindAtMs);
      }
      if (orderTotal > 0) out.event_order_score = orderHits / orderTotal;
      if (coords > 0) out.integer_coord_ratio = intCoords / coords;
      var nCells = Object.keys(cells).length;
      if (coords >= 4 && nCells > 0) {
        out.input_mouse_entropy = Math.min(1, nCells / Math.max(coords, 1));
      }
      if (speeds.length >= 3) {
        var mean = 0;
        for (var j = 0; j < speeds.length; j++) mean += speeds[j];
        mean /= speeds.length;
        var varSum = 0;
        for (var j2 = 0; j2 < speeds.length; j2++) {
          var dlt = speeds[j2] - mean;
          varSum += dlt * dlt;
        }
        var sd = Math.sqrt(varSum / speeds.length);
        out.input_velocity_cv = mean > 1e-9 ? sd / mean : null;
      }
      if (dwells.length >= 2) {
        var meanD = 0;
        for (var j3 = 0; j3 < dwells.length; j3++) meanD += dwells[j3];
        meanD /= dwells.length;
        var varD = 0;
        for (var j4 = 0; j4 < dwells.length; j4++) {
          var dd = dwells[j4] - meanD;
          varD += dd * dd;
        }
        out.key_dwell_stddev = Math.sqrt(varD / dwells.length);
      }
      // iss/39 R12: straightness = net displacement / path length; click interval CV
      if (pathLen > 1 && prevX != null && firstX != null) {
        var net = Math.sqrt((prevX - firstX) * (prevX - firstX) + (prevY - firstY) * (prevY - firstY));
        out.path_straightness = Math.max(0, Math.min(1, net / pathLen));
      }
      if (clickGaps.length >= 2) {
        var meanG = 0;
        for (var j5 = 0; j5 < clickGaps.length; j5++) meanG += clickGaps[j5];
        meanG /= clickGaps.length;
        var varG = 0;
        for (var j6 = 0; j6 < clickGaps.length; j6++) {
          var dg = clickGaps[j6] - meanG;
          varG += dg * dg;
        }
        var sdG = Math.sqrt(varG / clickGaps.length);
        out.click_interval_cv = meanG > 1e-9 ? sdG / meanG : null;
      }
      if (jerks.length >= 3) {
        var meanJ = 0;
        for (var jj = 0; jj < jerks.length; jj++) meanJ += Math.abs(jerks[jj]);
        meanJ /= jerks.length;
        var varJ = 0;
        for (var jj2 = 0; jj2 < jerks.length; jj2++) {
          var dj = Math.abs(jerks[jj2]) - meanJ;
          varJ += dj * dj;
        }
        var sdJ = Math.sqrt(varJ / jerks.length);
        out.input_jerk_cv = meanJ > 1e-12 ? sdJ / meanJ : 0;
      }
      if (curvSamples.length >= 2) {
        var csum = 0;
        for (var jc = 0; jc < curvSamples.length; jc++) csum += curvSamples[jc];
        out.path_curvature_mean = Math.round((csum / curvSamples.length) * 1e4) / 1e4;
      }
      out.overshoot_n = overshootN;
      if (fittsResiduals.length >= 1) {
        var fsum = 0;
        for (var jf = 0; jf < fittsResiduals.length; jf++) fsum += Math.abs(fittsResiduals[jf]);
        out.fitts_residual_mean = Math.round((fsum / fittsResiduals.length) * 1e4) / 1e4;
      }
      if (dwells.length >= 1) {
        var meanHold = 0;
        for (var jh = 0; jh < dwells.length; jh++) meanHold += dwells[jh];
        out.key_hold_mean_ms = meanHold / dwells.length;
      }
      if (flights.length >= 2) {
        var meanF = 0;
        for (var jfl = 0; jfl < flights.length; jfl++) meanF += flights[jfl];
        meanF /= flights.length;
        var varF = 0;
        for (var jfl2 = 0; jfl2 < flights.length; jfl2++) {
          var df = flights[jfl2] - meanF;
          varF += df * df;
        }
        var sdF = Math.sqrt(varF / flights.length);
        out.key_flight_cv = meanF > 1e-9 ? sdF / meanF : null;
      }
      if (winRisks.length) {
        var wr = 0;
        for (var jw = 0; jw < winRisks.length; jw++) if (winRisks[jw] > wr) wr = winRisks[jw];
        out.window_risk_max = wr;
      }
      // signal presence
      if (coords >= 4 || dwells.length >= 2 || scrollBurst > 0) {
        out.rpa_signal = "present";
      } else {
        out.rpa_signal = "thin";
      }
      // Rule-layer hard tells (001 heuristic layer)
      out.rpa_rule_teleport = false;
      out.rpa_rule_metronomic = false;
      out.rpa_rule_zero_jerk = false;
      if (speeds.length >= 4 && out.path_straightness != null && out.path_straightness > 0.98) {
        out.rpa_rule_teleport = out.path_straightness > 0.995;
      }
      if (out.input_velocity_cv != null && out.input_velocity_cv < 0.05) {
        out.rpa_rule_metronomic = true;
      }
      if (out.input_jerk_cv != null && out.input_jerk_cv < 0.02 && jerks.length >= 4) {
        out.rpa_rule_zero_jerk = true;
      }
      return out;
    }

    // iss/21 T-BIO-2: sensitive form path — zero pre-move before password/submit is a hard tell
    function markSensitiveAction(tag) {
      state.sensitiveActionSeen = true;
      var kin = kinematicsSummary(state.events);
      if ((kin.pre_action_move_count || 0) <= 0) {
        state.sensitiveZeroMove = true;
      }
      try {
        flush("sensitive_" + (tag || "action"), true);
      } catch (eS) {}
    }
    if (doc && doc.addEventListener) {
      try {
        doc.addEventListener(
          "submit",
          function () {
            markSensitiveAction("submit");
          },
          true
        );
        doc.addEventListener(
          "focusin",
          function (ev) {
            var el = ev && ev.target;
            if (!el) return;
            var typ = String((el.type || el.getAttribute && el.getAttribute("type") || "")).toLowerCase();
            var name = String(el.name || el.id || "").toLowerCase();
            var autocomplete = String(el.autocomplete || "").toLowerCase();
            if (
              typ === "password" ||
              typ === "email" ||
              autocomplete.indexOf("password") >= 0 ||
              autocomplete.indexOf("cc-") === 0 ||
              /password|passwd|card|cvv|otp/.test(name)
            ) {
              markSensitiveAction("focus_" + typ);
            }
          },
          true
        );
      } catch (eSens) {}
    }

    /** Privacy-minimized aggregate (iss/47). Default upload — no raw key/code/trajectory dump. */
    function buildFeaturesV2(kin, reason, hiding) {
      var evs = state.events;
      var types = {};
      var pointerN = 0;
      var keyN = 0;
      var scrollN = 0;
      for (var i = 0; i < evs.length; i++) {
        var e = evs[i] || {};
        var k = String(e.kind || e.type || "unk");
        types[k] = (types[k] || 0) + 1;
        if (/pointer|mouse|touch|click/.test(k)) pointerN++;
        if (/key/.test(k)) keyN++;
        if (/scroll|wheel/.test(k)) scrollN++;
      }
      var typeKeys = Object.keys(types);
      var durMs =
        evs.length >= 2
          ? Math.max(0, (evs[evs.length - 1].t || 0) - (evs[0].t || 0))
          : 0;
      var durBucket =
        durMs < 1000
          ? "0_1s"
          : durMs < 5000
            ? "1_5s"
            : durMs < 15000
              ? "5_15s"
              : durMs < 60000
                ? "15_60s"
                : "60s_plus";
      var sampleQuality =
        evs.length >= 12 && typeKeys.length >= 3
          ? "adequate"
          : evs.length >= 4
            ? "thin"
            : "insufficient";
      // Segment families: behavior vs control-plane are scored separately server-side.
      // Count alone must not invent human (server enforces; FE marks family).
      return {
        schema: "rpa_features_v2",
        schema_version: 2,
        page_id: state.page_id,
        page_path: state.page_path || pagePathKey(state.page_url),
        tab_window_id: state.tab_window_id || null,
        page_scope: "url_path+tab_window",
        segment: {
          kind: hiding ? "page_unload" : "rolling_window",
          pointer_type: pointerN >= keyN ? "pointer" : "keyboard",
          n_events: evs.length,
          duration_bucket: durBucket,
          sample_quality: sampleQuality,
          family: "behavior",
          count_not_safety: true,
        },
        features: {
          n_events: evs.length,
          type_diversity_n: typeKeys.length,
          pointer_n: pointerN,
          key_n: keyN,
          scroll_n: scrollN,
          input_mouse_entropy: kin.input_mouse_entropy,
          integer_coord_ratio: kin.integer_coord_ratio,
          key_dwell_stddev: kin.key_dwell_stddev,
          key_flight_cv: kin.key_flight_cv,
          key_hold_mean_ms: kin.key_hold_mean_ms,
          pre_action_move_count: kin.pre_action_move_count,
          ttfi_ms: kin.ttfi_ms,
          event_order_score: kin.event_order_score,
          scroll_burst_n: kin.scroll_burst_n,
          input_velocity_cv: kin.input_velocity_cv,
          input_jerk_cv: kin.input_jerk_cv,
          path_length_px: kin.path_length_px,
          path_straightness: kin.path_straightness != null ? kin.path_straightness : null,
          path_curvature_mean: kin.path_curvature_mean,
          click_interval_cv: kin.click_interval_cv != null ? kin.click_interval_cv : null,
          overshoot_n: kin.overshoot_n,
          fitts_residual_mean: kin.fitts_residual_mean,
          window_risk_max: kin.window_risk_max,
          rpa_signal: kin.rpa_signal || "none",
          rpa_rule_teleport: !!kin.rpa_rule_teleport,
          rpa_rule_metronomic: !!kin.rpa_rule_metronomic,
          rpa_rule_zero_jerk: !!kin.rpa_rule_zero_jerk,
          sensitive_action_zero_move: !!kin.sensitive_action_zero_move,
          sensitive_action_seen: !!state.sensitiveActionSeen,
        },
        privacy: {
          raw_events_uploaded: false,
          raw_keys_uploaded: false,
          full_url_uploaded: false,
          coord_bucket_px: 4,
        },
        flush_reason: reason || "tick",
      };
    }

    function flush(reason, force) {
      var now = Date.now();
      var hiding = reason === "pagehide" || reason === "hidden";
      var slice;
      if (state.inflightSeg && state.inflightSeg.events && state.inflightSeg.events.length) {
        slice = state.inflightSeg.events;
      } else {
        slice = [];
        for (var si = 0; si < state.events.length; si++) {
          var evs = state.events[si] || {};
          var seq = Number(evs.event_seq) || 0;
          if (seq > (state.ackedSeq || 0)) slice.push(evs);
        }
      }
      if (!slice.length && !force && !hiding) return null;
      var seqStart = slice.length ? Number(slice[0].event_seq) || (state.ackedSeq + 1) : state.ackedSeq + 1;
      var seqEnd = slice.length
        ? Number(slice[slice.length - 1].event_seq) || seqStart
        : seqStart;
      var segmentId =
        (state.inflightSeg && state.inflightSeg.segment_id) ||
        String(state.page_id || "p") + "|" + seqStart + "-" + seqEnd;
      state.inflightSeg = { events: slice, seq_start: seqStart, seq_end: seqEnd, segment_id: segmentId };
      var kin = kinematicsSummary(slice);
      var feat = buildFeaturesV2(kin, reason, hiding);
      // Default: aggregate only (iss/47). Raw stream only if lab explicitly enables.
      var uploadRaw = false;
      try {
        uploadRaw = !!(win.__GR_RPA_UPLOAD_RAW__ || win.localStorage.getItem("gr_rpa_raw") === "1");
      } catch (eRaw) {}
      var fields = {
        behavior_early_bound: true,
        behavior_count: slice.length,
        rpa_seq_start: seqStart,
        rpa_seq_end: seqEnd,
        rpa_segment_id: segmentId,
        rpa_acked_seq: state.ackedSeq || 0,
        pagehide_flush: !!hiding,
        behavior_pagehide: !!hiding,
        rpa_flush_reason: reason || "tick",
        // Idle arm: FE timer = RPA_IDLE_MS (30s = server RPA_IDLE_ANALYZE_MS). Reasons: idle_30s.
        rpa_idle_flush: reason === "idle_30s" || reason === "idle_45s",
        page_id: state.page_id,
        // path only — not full URL+query by default
        page_path: state.page_path || pagePathKey(state.page_url),
        tab_window_id: state.tab_window_id || null,
        page_scope: "url_path+tab_window",
        collected_at: now,
        rpa_source: source,
        sandbox_kind: source === "main" ? "main" : source.split(":")[0] || source,
        rpa_features_v2: feat,
        // Flatten top features for server scorers that read flat keys
        input_mouse_entropy: kin.input_mouse_entropy,
        integer_coord_ratio: kin.integer_coord_ratio,
        key_dwell_stddev: kin.key_dwell_stddev,
        pre_action_move_count: kin.pre_action_move_count,
        behavior_type_diversity_n: kin.behavior_type_diversity_n,
        ttfi_ms: kin.ttfi_ms,
        event_order_score: kin.event_order_score,
        scroll_burst_n: kin.scroll_burst_n,
        input_velocity_cv: kin.input_velocity_cv,
        path_length_px: kin.path_length_px,
        sensitive_action_zero_move: !!kin.sensitive_action_zero_move,
        sensitive_action_seen: !!state.sensitiveActionSeen,
        // P0/P1 goal fields for G4 (RPA) — structured, no raw coords
        input_modality: (function () {
          try {
            var hasTouch = false;
            var hasMouse = false;
            for (var i = 0; i < slice.length; i++) {
              var k = String(slice[i].kind || slice[i].type || "");
              if (k.indexOf("touch") === 0 || k === "pointerdown") hasTouch = hasTouch || k.indexOf("touch") === 0;
              if (k.indexOf("mouse") === 0 || k === "click" || k.indexOf("pointer") === 0) hasMouse = true;
            }
            var mtp = 0;
            try {
              mtp = Number(navigator.maxTouchPoints) || 0;
            } catch (eM) {}
            if (hasTouch && hasMouse) return "hybrid";
            if (hasTouch || mtp > 0) return "touch";
            if (hasMouse) return "mouse";
            return "unknown";
          } catch (eI) {
            return "unknown";
          }
        })(),
        rpa_coalesced_stats: (function () {
          var n = 0;
          var samples = 0;
          try {
            for (var i = 0; i < slice.length; i++) {
              var ev = slice[i] || {};
              if (typeof ev.coalesced_n === "number") {
                samples++;
                n += ev.coalesced_n;
              }
            }
          } catch (eC) {}
          return {
            events_with_coalesced: samples,
            coalesced_total: n,
            // true when pointer API exists but no coalesced samples ever (synthetic risk hint)
            no_coalesced_observed: samples === 0,
          };
        })(),
        rpa_force_flush: !!force || !!hiding,
      };
      if (uploadRaw) {
        fields.behavior_events = slice.slice();
        fields.page_url = state.page_url || pageUrlDefault();
        fields.rpa_raw_lab_only = true;
      }
      state.lastUploadMs = now;

      if (typeof opts.post === "function") {
        try {
          opts.post(fields, reason, force);
        } catch (eP) {}
        return fields;
      }
      if (opts.queue && typeof opts.queue.enqueue === "function") {
        try {
          // Cycle closed / 410 halt — do not enqueue B11 (was flooding console 410s).
          if (
            win.__GR_HALT_UPLOADS__ ||
            win.__GR_CYCLE_CLOSED__ ||
            win.__GR_STOP_PROBE__ ||
            (opts.queue.isHalted && opts.queue.isHalted(state.session_id))
          ) {
            return fields;
          }
          opts.queue.enqueue({
            session_id: state.session_id,
            batch_id: "B11_interaction",
            source: source,
            inject_path: state.inject_path,
            priority: hiding ? 100 : 88,
            force: !!force || hiding,
            rpa_seq_start: seqStart,
            rpa_seq_end: seqEnd,
            rpa_segment_id: segmentId,
            payload: { fields: fields, sandbox_kind: fields.sandbox_kind },
          });
        } catch (eQ) {}
      }
      return fields;
    }

    if (doc && doc.addEventListener) {
      [
        "pointerdown",
        "pointermove",
        "pointerup",
        "scroll",
        "keydown",
        "keyup",
        "click",
        "touchstart",
        "touchmove",
        "wheel",
        "mousemove",
      ].forEach(function (type) {
        try {
          doc.addEventListener(
            type,
            function (ev) {
              push(type, ev);
            },
            { passive: true, capture: true }
          );
        } catch (eL) {}
      });
    }

    // Continuous flush tick (not route_plan gated) — coalesce for completeness without spam.
    state.contFlushN = 0;
    var tick = setInterval(function () {
      // Only true unload stops RPA; background continues (maximize multi-window).
      if (win.__GR_PAGE_UNLOADING__ || (win.__GR_PAGE_HIDING__ && source === "main" && win.__GR_PAGE_UNLOADING__)) {
        return;
      }
      if (
        win.__GR_HALT_UPLOADS__ ||
        win.__GR_CYCLE_CLOSED__ ||
        win.__GR_STOP_PROBE__
      ) {
        return;
      }
      var now = Date.now();
      var lastEv = state.lastEventMs || 0;
      var lastUp = state.lastUploadMs || 0;
      if (state.contFlushN >= CONT_FLUSH_MAX) {
        if (
          state.events.length &&
          lastUp &&
          now - lastUp >= RPA_IDLE_MS &&
          now - lastEv >= RPA_IDLE_MS
        ) {
          flush("idle_30s", false);
        }
        return;
      }
      if (state.events.length && lastEv > lastUp && now - lastUp >= CONT_FLUSH_MS) {
        state.contFlushN += 1;
        flush("continuous", false);
      } else if (
        state.events.length &&
        lastUp &&
        now - lastUp >= RPA_IDLE_MS &&
        now - lastEv >= RPA_IDLE_MS
      ) {
        flush("idle_30s", false);
      }
    }, 2000);
    state._tick = tick;

    function flushUnload() {
      try {
        if (source === "main") {
          win.__GR_PAGE_UNLOADING__ = true;
          win.__GR_PAGE_HIDING__ = true;
        }
      } catch (e) {}
      flush("pagehide", true);
    }
    function flushBackground() {
      try {
        if (source === "main") {
          win.__GR_PAGE_BACKGROUNDED__ = true;
          // Do NOT set PAGE_HIDING — probing continues in unfocused windows.
          if (!win.__GR_PAGE_UNLOADING__) win.__GR_PAGE_HIDING__ = false;
        }
      } catch (e) {}
      flush("hidden", true);
    }
    try {
      win.addEventListener("pagehide", flushUnload);
      win.addEventListener("beforeunload", flushUnload);
      if (doc) {
        doc.addEventListener("visibilitychange", function () {
          if (doc.visibilityState === "hidden") flushBackground();
          else if (doc.visibilityState === "visible") {
            try {
              win.__GR_PAGE_BACKGROUNDED__ = false;
              if (!win.__GR_PAGE_UNLOADING__) win.__GR_PAGE_HIDING__ = false;
            } catch (eV) {}
          }
        });
      }
    } catch (eB) {}

    // Initial bind upload so server knows monitor is live
    flush("bind", false);
    win[boundKey] = state;
    return state;
  }

  /**
   * Worker-side continuous monitor (no DOM pointers).
   * Records worker ticks / message activity — honest capability for dedicated workers.
   */
  function bindWorker(opts) {
    opts = opts || {};
    var source = opts.source || "worker:d1";
    if (global.__GR_RPA_WORKER_BOUND__) return global.__GR_RPA_WORKER_STATE__;
    global.__GR_RPA_WORKER_BOUND__ = true;
    var state = {
      source: source,
      events: [],
      lastEventMs: 0,
      lastUploadMs: 0,
      nextSeq: 1,
      ackedSeq: 0,
      inflightSeg: null,
    };
    function push(kind, extra) {
      dualTrackAdmit(state.events, kind);
      var row = {
        t: Date.now(),
        kind: kind,
        type: kind,
        source: source,
        event_seq: state.nextSeq++,
        _track: isMoveKind(kind) ? "move" : "action",
        non_biometric_execution: true,
      };
      if (extra) {
        Object.keys(extra).forEach(function (k) {
          row[k] = extra[k];
        });
      }
      state.events.push(row);
      state.lastEventMs = Date.now();
    }
    function flush(reason) {
      var slice;
      if (state.inflightSeg && state.inflightSeg.events && state.inflightSeg.events.length) {
        slice = state.inflightSeg.events;
      } else {
        slice = [];
        for (var si = 0; si < state.events.length; si++) {
          var evs = state.events[si] || {};
          var seq = Number(evs.event_seq) || 0;
          if (seq > (state.ackedSeq || 0)) slice.push(evs);
        }
      }
      var seqStart = slice.length
        ? Number(slice[0].event_seq) || (state.ackedSeq + 1)
        : state.ackedSeq + 1;
      var seqEnd = slice.length
        ? Number(slice[slice.length - 1].event_seq) || seqStart
        : seqStart;
      var segmentId =
        (state.inflightSeg && state.inflightSeg.segment_id) ||
        "worker|" + seqStart + "-" + seqEnd;
      state.inflightSeg = {
        events: slice,
        seq_start: seqStart,
        seq_end: seqEnd,
        segment_id: segmentId,
      };
      var fields = {
        behavior_early_bound: true,
        behavior_events: slice,
        behavior_count: slice.length,
        rpa_seq_start: seqStart,
        rpa_seq_end: seqEnd,
        rpa_segment_id: segmentId,
        rpa_acked_seq: state.ackedSeq || 0,
        pagehide_flush: reason === "close",
        rpa_flush_reason: reason || "tick",
        rpa_idle_flush: reason === "idle_30s" || reason === "idle_45s",
        collected_at: Date.now(),
        rpa_source: source,
        sandbox_kind: "worker",
        worker_rpa: true,
        // iss/47: worker ticks are execution surface, not biometric path
        non_biometric_execution: true,
      };
      state.lastUploadMs = Date.now();
      if (typeof opts.postMessage === "function") {
        opts.postMessage({
          from: "gr_worker_rpa",
          type: "rpa_flush",
          source: source,
          ok: true,
          payload: fields,
        });
      } else if (typeof self !== "undefined" && self.postMessage) {
        self.postMessage({
          from: "gr_worker_rpa",
          type: "rpa_flush",
          source: source,
          ok: true,
          payload: fields,
        });
      }
      return fields;
    }
    // Message activity
    try {
      self.addEventListener("message", function (ev) {
        push("worker_message", { data_type: typeof (ev && ev.data) });
      });
    } catch (e) {}
    // Heartbeat ticks (scheduler / timer surface)
    var n = 0;
    setInterval(function () {
      n++;
      push("worker_tick", { n: n });
      var now = Date.now();
      if (state.events.length && now - (state.lastUploadMs || 0) >= CONT_FLUSH_MS) {
        flush("continuous");
      } else if (
        state.lastUploadMs &&
        now - state.lastUploadMs >= RPA_IDLE_MS &&
        now - state.lastEventMs >= RPA_IDLE_MS
      ) {
        flush("idle_30s");
      }
    }, 1500);
    flush("bind");
    global.__GR_RPA_WORKER_STATE__ = state;
    return state;
  }

  // Export pure helpers for node harness / unit tests (shipped entry still binds globals).
  global.GRRpaHelpers = {
    makePageId: makePageId,
    pagePathKey: pagePathKey,
    tabWindowId: tabWindowId,
    keyClass: keyClass,
    RPA_IDLE_MS: RPA_IDLE_MS,
    EVENT_CAP: EVENT_CAP,
    MOVE_CAP: MOVE_CAP,
    ACTION_CAP: ACTION_CAP,
    isMoveKind: isMoveKind,
    dualTrackAdmit: dualTrackAdmit,
    /** Pure flush-field builder for tests (no DOM/queue). */
    buildFlushFields: function (opts) {
      opts = opts || {};
      var reason = opts.reason || "tick";
      var events = opts.events || [];
      var pageId = opts.page_id || makePageId(opts.page_url || "https://example.com/p", {
        sessionStorage: opts.sessionStorage,
      });
      var hiding = reason === "pagehide" || reason === "hidden";
      var idle = reason === "idle_45s" || reason === "idle_30s";
      // Minimal kinematics from event list
      var types = {};
      for (var i = 0; i < events.length; i++) {
        var k = String((events[i] && (events[i].kind || events[i].type)) || "unk");
        types[k] = 1;
      }
      var typeN = Object.keys(types).length;
      var feat = {
        schema: "rpa_features_v2",
        schema_version: 1,
        page_id: pageId,
        page_path: pagePathKey(opts.page_url || "https://example.com/p"),
        segment: {
          kind: hiding ? "page_unload" : idle ? "idle_window" : "rolling_window",
          n_events: events.length,
          sample_quality:
            events.length >= 12 && typeN >= 3
              ? "adequate"
              : events.length >= 4
                ? "thin"
                : "insufficient",
        },
        features: {
          n_events: events.length,
          type_diversity_n: typeN,
        },
        privacy: {
          raw_events_uploaded: false,
          raw_keys_uploaded: false,
          full_url_uploaded: false,
        },
        flush_reason: reason,
      };
      var fields = {
        behavior_early_bound: true,
        behavior_count: events.length,
        pagehide_flush: !!hiding,
        behavior_pagehide: !!hiding,
        rpa_flush_reason: reason,
        rpa_idle_flush: idle,
        page_id: pageId,
        page_path: pagePathKey(opts.page_url || "https://example.com/p"),
        page_scope: "url_path+tab_window",
        rpa_features_v2: feat,
      };
      // Default: no raw events, no key/code
      if (opts.uploadRaw) {
        fields.behavior_events = events;
      }
      return fields;
    },
  };

  global.GRRpaMonitor = {
    bind: bind,
    bindWorker: bindWorker,
    RPA_IDLE_MS: RPA_IDLE_MS,
    EVENT_CAP: EVENT_CAP,
    MOVE_CAP: MOVE_CAP,
    ACTION_CAP: ACTION_CAP,
    helpers: global.GRRpaHelpers,
    ackSegment: function (source, seqEnd, segmentId) {
      try {
        var end = Number(seqEnd) || 0;
        var apply = function (st) {
          if (!st) return;
          if (end > (st.ackedSeq || 0)) st.ackedSeq = end;
          if (
            !st.inflightSeg ||
            !segmentId ||
            String(st.inflightSeg.segment_id) === String(segmentId) ||
            Number(st.inflightSeg.seq_end) <= end
          ) {
            st.inflightSeg = null;
          }
        };
        apply(global.__GR_RPA_WORKER_STATE__);
        var w = typeof window !== "undefined" ? window : global;
        Object.keys(w).forEach(function (k) {
          if (k.indexOf("__GR_RPA_BOUND_") === 0) apply(w[k]);
        });
      } catch (eAck) {}
    },
  };

  // UMD-ish for worker importScripts
  if (typeof module !== "undefined" && module.exports) {
    module.exports = global.GRRpaMonitor;
  }
})(typeof self !== "undefined" ? self : typeof window !== "undefined" ? window : globalThis);
