/**
 * GR WebGL / console governor — process-wide.
 *
 * Problems this solves:
 * 1) Chrome: "Too many active WebGL contexts"
 * 2) Firefox: flood of "WebGL context was lost" when loseContext is called every pack
 * 3) Deprecated getEntriesByType / INVALID_ENUM / WebGPU adapter spam
 *
 * Strategy (product):
 * - Detached probe canvases share ONE live WebGL context per type (webgl / webgl2).
 * - Soft release does NOT call loseContext (avoids Firefox log thrash).
 * - Hard lose only on idle timeout, pagehide, or explicit forceCap under pressure.
 * - In-DOM business canvases still capped at MAX_LIVE=1.
 */
(function (global) {
  "use strict";
  if (global.__GR_GL_GOV_INSTALLED__) return;
  global.__GR_GL_GOV_INSTALLED__ = 1;

  // ---- Permanent console noise filter (our probes trigger browser-native spam) ----
  // Must stay for page life: temporary mutes miss async GPU/WebGL logs.
  try {
    if (!global.__GR_CONSOLE_NOISE_FILTER__) {
      global.__GR_CONSOLE_NOISE_FILTER__ = 1;
      var _c = global.console;
      if (_c) {
        var _NOISE =
          /WebGPU is experimental|Failed to create WebGPU Context Provider|No available adapters|gpuweb\/wiki\/Implementation|INVALID_ENUM|INVALID_OPERATION|loseContext|context already lost|context was lost|InstallTrigger is deprecated|Fingerprinting Protection is altering|Window\.fullScreen attribute is deprecated|onmozfullscreen|AudioContext was prevented from starting|orientation sensor is deprecated|motion sensor is deprecated|Drawing to a destination rect smaller|Out-of-bounds reads with readPixels|\[NEW\] Explain Console|favicon\.ico|ResizeObserver loop|CORS policy.*gateway\/early|net::ERR_ABORTED/i;
        function wrapNoise(meth) {
          if (typeof _c[meth] !== "function") return;
          var orig = _c[meth].bind(_c);
          _c[meth] = function () {
            try {
              var s = Array.prototype.map
                .call(arguments || [], function (a) {
                  if (a == null) return "";
                  if (typeof a === "string") return a;
                  if (a && a.message) return String(a.message);
                  return String(a);
                })
                .join(" ");
              if (_NOISE.test(s)) return;
            } catch (eF) {}
            return orig.apply(_c, arguments);
          };
        }
        ["warn", "error", "info", "log", "debug"].forEach(wrapNoise);
      }
    }
  } catch (eNoise) {}

  // Allow webgl + webgl2 shared slots; probes never need more.
  var MAX_LIVE = 2;
  var live = [];
  var stats = {
    creates: 0,
    reused: 0,
    forced_lose: 0,
    soft_release: 0,
    patches: 0,
    blocked: 0,
    shared_hits: 0,
    lose_noop: 0,
  };
  var lastCreateAt = 0;
  var MIN_CREATE_GAP_MS = 16;
  // Hard lose disabled during probe (Firefox "context was lost" spam).
  // Only pagehide/beforeunload sets allowHardLose=true briefly.
  var allowHardLose = false;
  var hardLoseTimer = null;
  // Shared pool for off-DOM probe canvases: type -> { canvas, gl, attrsKey }
  var pool = Object.create(null);
  var debugInfoCache = null; // { vendor, renderer }

  function isWebGlType(type) {
    if (!type) return false;
    var t = String(type).toLowerCase();
    return (
      t === "webgl" ||
      t === "webgl2" ||
      t === "experimental-webgl" ||
      t === "experimental-webgl2"
    );
  }

  function poolKey(type) {
    var t = String(type || "").toLowerCase();
    if (t.indexOf("webgl2") >= 0) return "webgl2";
    return "webgl";
  }

  function isDetachedCanvas(el) {
    try {
      if (!el) return true;
      if (el.parentNode) return false;
      // OffscreenCanvas has no parentNode
      if (typeof OffscreenCanvas !== "undefined" && el instanceof OffscreenCanvas) return true;
      return true;
    } catch (e) {
      return true;
    }
  }

  function hardLoseOne(entry) {
    if (!entry) return;
    // Soft-only path during probe: never call loseContext (Firefox logs every call).
    if (!allowHardLose) {
      softClearOne(entry);
      stats.lose_noop++;
      return;
    }
    try {
      var g = entry.gl;
      if (g) {
        // Already lost — skip (avoids "loseContext: Already lost" console spam).
        try {
          if (typeof g.isContextLost === "function" && g.isContextLost()) {
            stats.lose_noop++;
            return;
          }
        } catch (eLost) {}
        try {
          // Prefer raw unwrapped lose if present
          var lose =
            (g.__gr_raw_lose_ext && g.__gr_raw_lose_ext) ||
            (g.getExtension && g.getExtension("WEBGL_lose_context"));
          if (lose && lose.__gr_raw_loseContext) {
            try {
              if (typeof g.isContextLost === "function" && g.isContextLost()) {
                stats.lose_noop++;
                return;
              }
            } catch (eL0) {}
            lose.__gr_raw_loseContext();
          } else if (lose && lose.loseContext) {
            lose.loseContext();
          }
        } catch (eL) {}
        try {
          if (g.useProgram) g.useProgram(null);
        } catch (eU) {}
      }
    } catch (e0) {}
    try {
      if (entry.canvas) {
        try {
          entry.canvas.width = 1;
          entry.canvas.height = 1;
        } catch (eSz) {}
      }
    } catch (eC) {}
    stats.forced_lose++;
  }

  function softClearOne(entry) {
    if (!entry || !entry.gl) return;
    try {
      if (entry.gl.useProgram) entry.gl.useProgram(null);
    } catch (eU) {}
    try {
      if (entry.gl.bindBuffer) {
        entry.gl.bindBuffer(entry.gl.ARRAY_BUFFER, null);
        entry.gl.bindBuffer(entry.gl.ELEMENT_ARRAY_BUFFER, null);
      }
    } catch (eB) {}
    stats.soft_release++;
  }

  function pruneDead() {
    live = live.filter(function (e) {
      try {
        if (!e || !e.gl) return false;
        if (typeof e.gl.isContextLost === "function" && e.gl.isContextLost()) return false;
        return true;
      } catch (eX) {
        return false;
      }
    });
    Object.keys(pool).forEach(function (k) {
      var s = pool[k];
      try {
        if (!s || !s.gl || (s.gl.isContextLost && s.gl.isContextLost())) {
          delete pool[k];
        }
      } catch (eP) {
        delete pool[k];
      }
    });
  }

  function cancelHardLoseTimer() {
    if (hardLoseTimer) {
      try {
        clearTimeout(hardLoseTimer);
      } catch (e) {}
      hardLoseTimer = null;
    }
  }

  function scheduleHardLose() {
    // Intentionally no-op during session — hard lose only on pagehide.
    cancelHardLoseTimer();
  }

  function drainLegacyLists(hard) {
    try {
      if (global.__GR_PROBE_GL__ && global.__GR_PROBE_GL__.length) {
        var list = global.__GR_PROBE_GL__;
        while (list.length) {
          var ent = list.shift();
          if (hard) hardLoseOne({ gl: ent && ent.gl, canvas: ent && ent.canvas });
          else softClearOne({ gl: ent && ent.gl, canvas: ent && ent.canvas });
        }
      }
      if (hard) {
        global.__GR_PROBE_GL__ = [];
        global.__GR_PROBE_CANVASES__ = [];
        global.__GR_LAST_GL__ = null;
      }
    } catch (eT) {}
  }

  /** Soft: free GPU bindings, keep context alive for reuse (no "context was lost" log). */
  function softReleaseAll() {
    pruneDead();
    live.forEach(softClearOne);
    drainLegacyLists(false);
    scheduleHardLose();
  }

  /** Hard: loseContext everything (Chrome context cap + idle cleanup). */
  function hardReleaseAll() {
    cancelHardLoseTimer();
    pruneDead();
    while (live.length) {
      hardLoseOne(live.shift());
    }
    live = [];
    Object.keys(pool).forEach(function (k) {
      var s = pool[k];
      if (s) hardLoseOne(s);
      delete pool[k];
    });
    drainLegacyLists(true);
    try {
      global.__GR_PROBE_GL__ = [];
      global.__GR_PROBE_CANVASES__ = [];
      global.__GR_LAST_GL__ = null;
    } catch (e0) {}
  }

  // Public default release = soft (product quiet). hard available as forceCap/releaseHard.
  function releaseAll() {
    softReleaseAll();
  }

  function forceCap() {
    pruneDead();
    while (live.length > MAX_LIVE) {
      var old = live.shift();
      hardLoseOne(old);
    }
    // Also drop pool entries that are dead
    pruneDead();
  }

  function track(gl, canvas) {
    if (!gl) return;
    pruneDead();
    for (var i = 0; i < live.length; i++) {
      if (live[i].gl === gl) {
        stats.reused++;
        return;
      }
    }
    while (live.length >= MAX_LIVE) {
      hardLoseOne(live.shift());
    }
    live.push({ gl: gl, canvas: canvas, at: Date.now() });
    try {
      global.__GR_LAST_GL__ = gl;
      global.__GR_PROBE_GL__ = [{ gl: gl, canvas: canvas }];
      global.__GR_PROBE_CANVASES__ = canvas ? [canvas] : [];
    } catch (eR) {}
  }

  function getShared(type, attrs, w, h, origGet, canvasThis) {
    cancelHardLoseTimer();
    var key = poolKey(type);
    var slot = pool[key];
    if (slot && slot.gl && !(slot.gl.isContextLost && slot.gl.isContextLost())) {
      try {
        if (w > 0) slot.canvas.width = w;
        if (h > 0) slot.canvas.height = h;
      } catch (eR) {}
      stats.reused++;
      stats.shared_hits++;
      track(slot.gl, slot.canvas);
      return slot.gl;
    }
    // Need create once for this type
    forceCap();
    var c = document.createElement("canvas");
    try {
      c.width = w > 0 ? w : canvasThis && canvasThis.width ? canvasThis.width : 256;
      c.height = h > 0 ? h : canvasThis && canvasThis.height ? canvasThis.height : 256;
    } catch (eS) {
      c.width = 256;
      c.height = 256;
    }
    var gl = null;
    try {
      gl = origGet.call(c, type, attrs);
    } catch (eG) {
      gl = null;
    }
    if (!gl && key === "webgl") {
      try {
        gl = origGet.call(c, "experimental-webgl", attrs);
      } catch (eE) {}
    }
    if (gl) {
      stats.creates++;
      lastCreateAt = Date.now();
      pool[key] = { canvas: c, gl: gl, type: type };
      track(gl, c);
    }
    return gl;
  }

  function installOn(Proto) {
    if (!Proto || !Proto.getContext || Proto.__gr_gl_gov__) return;
    var orig = Proto.getContext;
    Proto.getContext = function (type, attrs) {
      if (!isWebGlType(type)) {
        return orig.apply(this, arguments);
      }
      cancelHardLoseTimer();
      // Detached probe canvases → shared pool (no per-pack lose/create thrash).
      if (isDetachedCanvas(this)) {
        var w = 0;
        var h = 0;
        try {
          w = this.width | 0;
          h = this.height | 0;
        } catch (eWh) {}
        return getShared(type, attrs, w, h, orig, this);
      }
      // In-DOM business canvas: hard-cap 1, may lose previous probe context once.
      forceCap();
      // If this canvas already has a context, native reuses.
      var gl = null;
      try {
        gl = orig.apply(this, arguments);
      } catch (eOrig) {
        gl = null;
      }
      if (gl) {
        stats.creates++;
        lastCreateAt = Date.now();
        track(gl, this);
        while (live.length > MAX_LIVE) {
          var drop = live.shift();
          if (drop && drop.gl !== gl) hardLoseOne(drop);
        }
      }
      return gl;
    };
    Proto.__gr_gl_gov__ = true;
    stats.patches++;
  }

  try {
    if (typeof HTMLCanvasElement !== "undefined") installOn(HTMLCanvasElement.prototype);
  } catch (eH) {}
  try {
    if (typeof OffscreenCanvas !== "undefined") installOn(OffscreenCanvas.prototype);
  } catch (eO) {}

  // ---- getParameter quiet (INVALID_ENUM) ----
  // Only forward known-safe pnames to native — unknown/R-pack enums return null
  // without calling gl (Firefox/Chrome log INVALID_ENUM before JS can catch).
  var GL1_SAFE = {
    0x1f00: 1, 0x1f01: 1, 0x1f02: 1, 0x8b8c: 1, // VENDOR RENDERER VERSION SHADING
    0x0d33: 1, 0x851c: 1, 0x84e8: 1, // MAX_TEXTURE/CUBE/RB
    0x8869: 1, 0x8dfb: 1, 0x8dfc: 1, 0x8dfd: 1, // MAX_VERTEX_* / FRAG
    0x8b4d: 1, 0x8b4c: 1, 0x8872: 1, // COMBINED / VERTEX_TEX / TEX_IMAGE
    0x0d50: 1, 0x0d52: 1, 0x0d53: 1, 0x0d54: 1, 0x0d55: 1, 0x0d56: 1, 0x0d57: 1, // bits
    0x0d3a: 1, 0x846e: 1, 0x846d: 1, // VIEWPORT_DIMS LINE POINT
    0x80a8: 1, 0x80a9: 1, // SAMPLE_BUFFERS SAMPLES
    0x0ba2: 1, 0x0b21: 1, 0x0b45: 1, 0x0b46: 1, // VIEWPORT LINE_WIDTH etc common state
    0x0c02: 1, 0x0b70: 1, 0x0c10: 1, 0x0b44: 1,
    0x8b8d: 1, 0x8b8b: 1, 0x9240: 1, 0x9241: 1, // CURRENT_PROGRAM / IMPLEMENTATION_COLOR_READ_*
  };
  // WebGL2-only (must have WebGL2 context)
  var GL2_SAFE = {
    0x80e8: 1, 0x80e9: 1, // MAX_ELEMENTS_VERTICES / INDICES
    0x84fd: 1, // MAX_TEXTURE_LOD_BIAS
    0x8cdf: 1, 0x8824: 1, 0x8d6b: 1, // DRAW_BUFFERS / COLOR_ATTACHMENTS / SAMPLES
    0x8a2e: 1, 0x8a2d: 1, 0x8a2f: 1, 0x8a2b: 1, 0x8a2c: 1,
    0x8a30: 1, 0x8a31: 1, 0x8a33: 1, 0x8a34: 1,
    0x8b4a: 1, 0x8b4b: 1, 0x8b49: 1,
    0x8904: 1, 0x8905: 1, 0x9111: 1, 0x9122: 1, 0x9125: 1,
    0x910e: 1, 0x910f: 1, 0x9110: 1,
    0x9315: 1, 0x9316: 1, 0x9317: 1,
    0x8e70: 1, 0x8c8a: 1, 0x8c8b: 1, 0x8c80: 1,
    0x88ff: 1, 0x8d57: 1, 0x8073: 1,
  };
  var EXT_PNAME = {
    0x84ff: ["EXT_texture_filter_anisotropic", "WEBKIT_EXT_texture_filter_anisotropic"],
    0x8fbb: ["EXT_disjoint_timer_query_webgl2", "EXT_disjoint_timer_query"],
    0x9245: ["WEBGL_debug_renderer_info"],
    0x9246: ["WEBGL_debug_renderer_info"],
  };
  var badPnameSeen = Object.create(null);

  function isWebGl2Ctx(gl) {
    try {
      return (
        (typeof WebGL2RenderingContext !== "undefined" && gl instanceof WebGL2RenderingContext) ||
        !!(gl && gl.MAX_3D_TEXTURE_SIZE && gl.drawArraysInstanced)
      );
    } catch (e) {
      return false;
    }
  }

  function patchGetParameter(Proto) {
    if (!Proto || !Proto.getParameter || Proto.__gr_gp_quiet__) return;
    var orig = Proto.getParameter;
    Proto.getParameter = function (pname) {
      if (pname == null || typeof pname !== "number" || !isFinite(pname)) return null;
      try {
        if (typeof this.isContextLost === "function" && this.isContextLost()) return null;
      } catch (eL) {}
      var key = pname | 0;
      if (badPnameSeen[key]) return null;
      // Prefer native UNMASKED via orig when cache missing/generic (iss/63 K).
      if ((key === 0x9245 || key === 0x9246) && debugInfoCache && debugInfoCache.native) {
        var cached = key === 0x9245 ? debugInfoCache.vendor : debugInfoCache.renderer;
        if (cached) return cached;
      }
      var w2 = isWebGl2Ctx(this);
      var allowed = GL1_SAFE[key] || (w2 && GL2_SAFE[key]) || EXT_PNAME[key];
      // Double-check: pname must appear as a constant on this gl (prevents cross-version INVALID_ENUM).
      if (allowed && !EXT_PNAME[key]) {
        var foundOnGl = false;
        try {
          // Fast path: common names
          if (
            this.VENDOR === key ||
            this.RENDERER === key ||
            this.VERSION === key ||
            this.SHADING_LANGUAGE_VERSION === key ||
            this.MAX_TEXTURE_SIZE === key ||
            this.MAX_RENDERBUFFER_SIZE === key ||
            this.MAX_VERTEX_ATTRIBS === key ||
            this.RED_BITS === key ||
            this.GREEN_BITS === key ||
            this.BLUE_BITS === key ||
            this.ALPHA_BITS === key ||
            this.DEPTH_BITS === key ||
            this.STENCIL_BITS === key ||
            this.ALIASED_LINE_WIDTH_RANGE === key ||
            this.ALIASED_POINT_SIZE_RANGE === key ||
            this.MAX_VIEWPORT_DIMS === key ||
            this.VIEWPORT === key ||
            this.MAX_CUBE_MAP_TEXTURE_SIZE === key ||
            this.MAX_COMBINED_TEXTURE_IMAGE_UNITS === key ||
            this.MAX_TEXTURE_IMAGE_UNITS === key ||
            this.MAX_VERTEX_TEXTURE_IMAGE_UNITS === key ||
            this.MAX_VERTEX_UNIFORM_VECTORS === key ||
            this.MAX_FRAGMENT_UNIFORM_VECTORS === key ||
            this.MAX_VARYING_VECTORS === key ||
            this.SAMPLE_BUFFERS === key ||
            this.SAMPLES === key ||
            this.CURRENT_PROGRAM === key ||
            this.LINE_WIDTH === key ||
            this.IMPLEMENTATION_COLOR_READ_TYPE === key ||
            this.IMPLEMENTATION_COLOR_READ_FORMAT === key
          ) {
            foundOnGl = true;
          } else if (w2) {
            foundOnGl = true; // GL2_SAFE already gated by isWebGl2Ctx
          }
        } catch (eF) {
          foundOnGl = !!allowed;
        }
        if (!foundOnGl && !w2) {
          // Not a known constant on this context — do not call native.
          badPnameSeen[key] = 1;
          stats.blocked++;
          return null;
        }
      }
      if (!allowed) {
        // Never call native with unknown enum (console INVALID_ENUM).
        badPnameSeen[key] = 1;
        stats.blocked++;
        return null;
      }
      var need = EXT_PNAME[key];
      if (need) {
        var got = false;
        var i;
        for (i = 0; i < need.length; i++) {
          try {
            if (this.getExtension && this.getExtension(need[i])) {
              got = true;
              break;
            }
          } catch (eX) {}
        }
        if (!got) return null;
      }
      var v = null;
      try {
        v = orig.call(this, pname);
      } catch (eG) {
        badPnameSeen[key] = 1;
        return null;
      }
      // Drain error; if INVALID_ENUM (0x0500) mark bad for next time (may still log once).
      try {
        var ge = this.getError && this.getError();
        if (ge === 0x0500) {
          badPnameSeen[key] = 1;
          return null;
        }
      } catch (eE) {}
      if ((key === 0x9245 || key === 0x9246) && v != null) {
        debugInfoCache = debugInfoCache || { vendor: "", renderer: "" };
        if (key === 0x9245) debugInfoCache.vendor = String(v);
        else debugInfoCache.renderer = String(v);
      }
      return v;
    };
    Proto.__gr_gp_quiet__ = true;
    stats.patches++;
  }

  // Quiet getExtension:
  // - WEBGL_debug_renderer_info: never native (Firefox deprecation)
  // - WEBGL_lose_context: wrap loseContext as soft no-op unless allowHardLose
  function patchGetExtension(Proto) {
    if (!Proto || !Proto.getExtension || Proto.__gr_ge_quiet__) return;
    var orig = Proto.getExtension;
    var origParam = Proto.getParameter;
    Proto.getExtension = function (name) {
      var n = String(name || "");
      if (n === "WEBGL_debug_renderer_info") {
        // iss/63 K: prefer *native* debug-info so UNMASKED_* are real GPU strings.
        // Never synthesize cache from VENDOR/RENDERER alone — that yields "WebKit WebGL"
        // and collapses model_key to unk:webkit_webgl on production.
        var nativeDbg = null;
        try {
          nativeDbg = orig.call(this, name);
        } catch (eNat) {
          nativeDbg = null;
        }
        if (nativeDbg) {
          try {
            if (origParam) {
              var uv = "";
              var ur = "";
              try {
                uv = String(
                  origParam.call(
                    this,
                    nativeDbg.UNMASKED_VENDOR_WEBGL != null
                      ? nativeDbg.UNMASKED_VENDOR_WEBGL
                      : 0x9245
                  ) || ""
                );
              } catch (eUv) {}
              try {
                ur = String(
                  origParam.call(
                    this,
                    nativeDbg.UNMASKED_RENDERER_WEBGL != null
                      ? nativeDbg.UNMASKED_RENDERER_WEBGL
                      : 0x9246
                  ) || ""
                );
              } catch (eUr) {}
              // Only cache when native unmasked looks real (not masked stack label).
              var urL = ur.toLowerCase();
              var generic =
                !ur ||
                urL === "webkit webgl" ||
                urL === "mozilla" ||
                urL === "firefox" ||
                urL.indexOf("webkit") === 0 && urL.indexOf("webgl") >= 0;
              if (!generic) {
                debugInfoCache = { vendor: uv, renderer: ur, native: true };
              }
            }
          } catch (eCache) {}
          return nativeDbg;
        }
        // No native extension (e.g. Firefox deprecation / blocked): soft shim.
        // Do NOT fill debugInfoCache from VENDOR/RENDERER — leave unmasked empty
        // so server can fall back to caps/class keys instead of fake WebKit labels.
        if (!debugInfoCache) {
          debugInfoCache = { vendor: "", renderer: "", native: false };
        }
        return {
          UNMASKED_VENDOR_WEBGL: 0x9245,
          UNMASKED_RENDERER_WEBGL: 0x9246,
        };
      }
      var ext = null;
      try {
        ext = orig.call(this, name);
      } catch (e) {
        ext = null;
      }
      if (ext && n === "WEBGL_lose_context" && ext.loseContext && !ext.__gr_lose_wrapped__) {
        try {
          var rawLose = ext.loseContext.bind(ext);
          var rawRestore = ext.restoreContext ? ext.restoreContext.bind(ext) : null;
          ext.__gr_raw_loseContext = rawLose;
          ext.loseContext = function () {
            // Product quiet: ignore mid-probe loses (collectors/nest still call this).
            if (!allowHardLose) {
              stats.lose_noop++;
              softClearOne({ gl: this && this.__gr_gl_ref, canvas: null });
              return;
            }
            try {
              var glRef = this.__gr_gl_ref;
              if (glRef && typeof glRef.isContextLost === "function" && glRef.isContextLost()) {
                stats.lose_noop++;
                return;
              }
            } catch (eAl) {}
            try {
              return rawLose();
            } catch (eRl) {
              stats.lose_noop++;
            }
          }.bind(ext);
          if (rawRestore) {
            ext.restoreContext = function () {
              try {
                return rawRestore();
              } catch (eR2) {}
            };
          }
          ext.__gr_lose_wrapped__ = 1;
          try {
            this.__gr_raw_lose_ext = ext;
          } catch (eRef) {}
        } catch (eW) {}
      }
      return ext;
    };
    Proto.__gr_ge_quiet__ = true;
    stats.patches++;
  }

  try {
    if (typeof WebGLRenderingContext !== "undefined") {
      patchGetParameter(WebGLRenderingContext.prototype);
      patchGetExtension(WebGLRenderingContext.prototype);
    }
  } catch (eW1) {}
  try {
    if (typeof WebGL2RenderingContext !== "undefined") {
      patchGetParameter(WebGL2RenderingContext.prototype);
      patchGetExtension(WebGL2RenderingContext.prototype);
    }
  } catch (eW2) {}

  // ---- Performance getEntriesByType quiet ----
  var SAFE_ENTRY_TYPES = { navigation: 1, resource: 1, mark: 1, measure: 1 };
  try {
    if (
      typeof Performance !== "undefined" &&
      Performance.prototype &&
      Performance.prototype.getEntriesByType &&
      !Performance.prototype.__gr_et_quiet__
    ) {
      var origEt = Performance.prototype.getEntriesByType;
      Performance.prototype.getEntriesByType = function (type) {
        var t = String(type || "");
        if (!SAFE_ENTRY_TYPES[t]) return [];
        try {
          return origEt.call(this, t) || [];
        } catch (e) {
          return [];
        }
      };
      Performance.prototype.__gr_et_quiet__ = true;
      stats.patches++;
    }
  } catch (eEt) {}

  // ---- WebGPU: single-flight + mute browser console spam during our call ----
  // Chrome/Edge emit "WebGPU is experimental" / "Failed to create WebGPU Context Provider"
  // on requestAdapter. We call orig once, cache null, filter those lines while calling.
  var _webgpuOrigRa = null;
  try {
    if (
      typeof navigator !== "undefined" &&
      navigator.gpu &&
      typeof navigator.gpu.requestAdapter === "function"
    ) {
      _webgpuOrigRa = navigator.gpu.requestAdapter.bind(navigator.gpu);
    }
  } catch (eWo) {}

  function muteWebGpuConsole(fn) {
    var c = global.console;
    if (!c || typeof fn !== "function") return typeof fn === "function" ? fn() : undefined;
    var oW = typeof c.warn === "function" ? c.warn.bind(c) : null;
    var oE = typeof c.error === "function" ? c.error.bind(c) : null;
    var oI = typeof c.info === "function" ? c.info.bind(c) : null;
    var oL = typeof c.log === "function" ? c.log.bind(c) : null;
    function isNoise(args) {
      try {
        var s = Array.prototype.map
          .call(args || [], function (a) {
            return a == null ? "" : typeof a === "string" ? a : a && a.message ? String(a.message) : String(a);
          })
          .join(" ");
        return /WebGPU|requestAdapter|Context Provider|gpuweb|Implementation-Status/i.test(s);
      } catch (eN) {
        return false;
      }
    }
    try {
      if (oW)
        c.warn = function () {
          if (isNoise(arguments)) return;
          return oW.apply(c, arguments);
        };
      if (oE)
        c.error = function () {
          if (isNoise(arguments)) return;
          return oE.apply(c, arguments);
        };
      if (oI)
        c.info = function () {
          if (isNoise(arguments)) return;
          return oI.apply(c, arguments);
        };
      if (oL)
        c.log = function () {
          if (isNoise(arguments)) return;
          return oL.apply(c, arguments);
        };
      return fn();
    } finally {
      try {
        if (oW) c.warn = oW;
        if (oE) c.error = oE;
        if (oI) c.info = oI;
        if (oL) c.log = oL;
      } catch (eR) {}
    }
  }

  /**
   * @returns {Promise<{adapter:object|null, skip:string|null, error?:string}>}
   */
  function requestWebGpuAdapterSafe(opts) {
    opts = opts || {};
    try {
      if (typeof isSecureContext !== "undefined" && !isSecureContext) {
        return Promise.resolve({ adapter: null, skip: "insecure_context" });
      }
      if (!_webgpuOrigRa && !(navigator.gpu && navigator.gpu.requestAdapter)) {
        return Promise.resolve({ adapter: null, skip: "no_gpu" });
      }
      if (global.__GR_WEBGPU_NO_ADAPTER__) {
        return Promise.resolve({ adapter: null, skip: "cached_none" });
      }
      if (global.__GR_WEBGPU_ADAPTER__ && !opts.powerPreference) {
        return Promise.resolve({ adapter: global.__GR_WEBGPU_ADAPTER__, skip: "cached" });
      }
      if (global.__GR_WEBGPU_ADAPTER_P__ && !opts.powerPreference) {
        return global.__GR_WEBGPU_ADAPTER_P__;
      }
      // Lab / headless: navigator.gpu exists but adapter always null — skip call after
      // first failure is cached. Optional opt-out of adapter probe entirely for clean console:
      if (global.__GR_WEBGPU_SKIP_ADAPTER__) {
        global.__GR_WEBGPU_NO_ADAPTER__ = 1;
        return Promise.resolve({ adapter: null, skip: "skipped_policy" });
      }
      var callOrig = _webgpuOrigRa;
      if (!callOrig) {
        try {
          callOrig = navigator.gpu.requestAdapter.bind(navigator.gpu);
        } catch (eB) {
          return Promise.resolve({ adapter: null, skip: "bind_fail" });
        }
      }
      var p = new Promise(function (resolve) {
        try {
          // Keep mute until promise settles — browser may log async after return.
          var oW2 = null;
          var oE2 = null;
          var oI2 = null;
          var oL2 = null;
          var c2 = global.console;
          function isNoise2(args) {
            try {
              var s = Array.prototype.map
                .call(args || [], function (a) {
                  return a == null
                    ? ""
                    : typeof a === "string"
                      ? a
                      : a && a.message
                        ? String(a.message)
                        : String(a);
                })
                .join(" ");
              return /WebGPU|requestAdapter|Context Provider|gpuweb|Implementation-Status/i.test(s);
            } catch (eN2) {
              return false;
            }
          }
          function restore() {
            try {
              if (c2 && oW2) c2.warn = oW2;
              if (c2 && oE2) c2.error = oE2;
              if (c2 && oI2) c2.info = oI2;
              if (c2 && oL2) c2.log = oL2;
            } catch (eRs) {}
          }
          if (c2) {
            oW2 = typeof c2.warn === "function" ? c2.warn.bind(c2) : null;
            oE2 = typeof c2.error === "function" ? c2.error.bind(c2) : null;
            oI2 = typeof c2.info === "function" ? c2.info.bind(c2) : null;
            oL2 = typeof c2.log === "function" ? c2.log.bind(c2) : null;
            if (oW2)
              c2.warn = function () {
                if (isNoise2(arguments)) return;
                return oW2.apply(c2, arguments);
              };
            if (oE2)
              c2.error = function () {
                if (isNoise2(arguments)) return;
                return oE2.apply(c2, arguments);
              };
            if (oI2)
              c2.info = function () {
                if (isNoise2(arguments)) return;
                return oI2.apply(c2, arguments);
              };
            if (oL2)
              c2.log = function () {
                if (isNoise2(arguments)) return;
                return oL2.apply(c2, arguments);
              };
          }
          // Keep mute 2s for async browser logs after resolve
          var muteUntil = Date.now() + 2000;
          function maybeRestore() {
            if (Date.now() >= muteUntil) restore();
            else setTimeout(maybeRestore, 100);
          }
          Promise.resolve(callOrig(opts))
            .then(function (ad) {
              if (!ad) {
                global.__GR_WEBGPU_NO_ADAPTER__ = 1;
                resolve({ adapter: null, skip: "null" });
              } else {
                if (!opts.powerPreference) global.__GR_WEBGPU_ADAPTER__ = ad;
                resolve({ adapter: ad, skip: null });
              }
              setTimeout(maybeRestore, 50);
            })
            .catch(function (e) {
              global.__GR_WEBGPU_NO_ADAPTER__ = 1;
              resolve({
                adapter: null,
                skip: "err",
                error: String((e && e.message) || e || "requestAdapter_failed"),
              });
              setTimeout(maybeRestore, 50);
            });
        } catch (e0) {
          global.__GR_WEBGPU_NO_ADAPTER__ = 1;
          resolve({ adapter: null, skip: "throw", error: String(e0 && e0.message) });
        }
      });
      if (!opts.powerPreference) global.__GR_WEBGPU_ADAPTER_P__ = p;
      return p;
    } catch (e1) {
      return Promise.resolve({ adapter: null, skip: "outer", error: String(e1 && e1.message) });
    }
  }

  try {
    if (_webgpuOrigRa && navigator.gpu && !navigator.gpu.__gr_ra_quiet__) {
      navigator.gpu.requestAdapter = function (opts) {
        return requestWebGpuAdapterSafe(opts || {}).then(function (r) {
          return r && r.adapter ? r.adapter : null;
        });
      };
      navigator.gpu.__gr_ra_quiet__ = true;
      stats.patches++;
    }
  } catch (eRa) {}

  // ---- AudioContext thrash quiet ----
  // Dense packs do new AudioContext() + close() per densify → autoplay spam and
  // "Can't close an AudioContext twice" when we share one instance.
  // Share one AC; close()/suspend() become safe no-ops (never destroy).
  try {
    var AC = global.AudioContext || global.webkitAudioContext;
    if (AC && !AC.__gr_ac_quiet__) {
      var OrigAC = AC;
      var sharedAc = null;
      function installSafeClose(ac) {
        if (!ac || ac.__gr_safe_close__) return ac;
        ac.__gr_safe_close__ = 1;
        var realClose = null;
        try {
          realClose = ac.close ? ac.close.bind(ac) : null;
        } catch (eC) {}
        ac.close = function () {
          // Do NOT destroy shared probe AC — only suspend if running.
          try {
            if (ac.state === "running" && typeof ac.suspend === "function") {
              return Promise.resolve(ac.suspend()).catch(function () {
                return undefined;
              });
            }
          } catch (eS) {}
          return Promise.resolve();
        };
        // Keep realClose for pagehide hard teardown only.
        ac.__gr_real_close__ = realClose;
        return ac;
      }
      var SharedAC = function () {
        if (sharedAc) {
          try {
            if (sharedAc.state === "closed") sharedAc = null;
          } catch (e) {
            sharedAc = null;
          }
        }
        if (sharedAc) return sharedAc;
        // Prefer OfflineAudioContext for sampleRate probes — no autoplay warning.
        try {
          var OAC = global.OfflineAudioContext || global.webkitOfflineAudioContext;
          if (OAC && !global.__GR_FORCE_LIVE_AC__) {
            var off = new OAC(1, 8, 44100);
            sharedAc = installSafeClose(off);
            try {
              sharedAc.sampleRate = off.sampleRate;
            } catch (eSr) {}
            return sharedAc;
          }
        } catch (eOff) {}
        try {
          sharedAc = installSafeClose(new OrigAC({ latencyHint: "playback" }));
          // Keep suspended — never auto-resume without gesture.
          try {
            if (sharedAc.state === "running" && sharedAc.suspend) sharedAc.suspend();
          } catch (eSus) {}
        } catch (eNew) {
          // Autoplay / construction failure — return a minimal stub so packs don't throw.
          sharedAc = installSafeClose({
            state: "suspended",
            sampleRate: 44100,
            baseLatency: 0,
            outputLatency: 0,
            destination: null,
            currentTime: 0,
            suspend: function () {
              return Promise.resolve();
            },
            resume: function () {
              return Promise.resolve();
            },
            close: function () {
              return Promise.resolve();
            },
          });
        }
        return sharedAc;
      };
      SharedAC.prototype = OrigAC.prototype;
      try {
        Object.setPrototypeOf(SharedAC, OrigAC);
      } catch (eP) {}
      SharedAC.__gr_ac_quiet__ = true;
      if (global.AudioContext) global.AudioContext = SharedAC;
      if (global.webkitAudioContext) global.webkitAudioContext = SharedAC;
      stats.patches++;
    }
  } catch (eAc) {}

  // pagehide → allow real lose once (page is going away; log spam irrelevant)
  try {
    if (typeof global.addEventListener === "function") {
      function hardOnLeave() {
        allowHardLose = true;
        try {
          hardReleaseAll();
        } finally {
          allowHardLose = false;
        }
      }
      global.addEventListener("pagehide", hardOnLeave, { capture: true });
      global.addEventListener("beforeunload", hardOnLeave, { capture: true });
    }
  } catch (ePh) {}

  /**
   * iss/46 H3: governor-cached UNMASKED_* is shared-pool / debug-info synthetic —
   * export as **governed_*** so product never confuses it with native unmasked.
   */
  function snapshotFields() {
    var ren = (debugInfoCache && debugInfoCache.renderer) || "";
    var ven = (debugInfoCache && debugInfoCache.vendor) || "";
    return {
      gl_governor_active: true,
      gl_governor_algo: "gr_gl_governor_v1",
      governed_webgl_renderer: ren || null,
      governed_webgl_vendor: ven || null,
      webgl_renderer_governed: ren || null,
      webgl_unmasked_is_governed: !!(ren || ven),
      // Keep native-named unmasked off this snapshot — collectors may still read GL;
      // product must prefer governed_* when gl_governor_active.
      gl_governor_shared_pool: Object.keys(pool).length > 0,
      gl_governor_live: live.length,
    };
  }

  /**
   * B10 / silicon hang recovery: temporarily allow real loseContext so a stuck
   * WebGL main-thread path can be torn down before self-heal re-kick.
   * Soft-only during normal probe; hard only for explicit hang recovery / pagehide.
   */
  function forceHardLoseForRetry(why) {
    var prev = allowHardLose;
    allowHardLose = true;
    cancelHardLoseTimer();
    try {
      hardReleaseAll();
    } catch (eH) {}
    // Drop shared pool so next B10 acquires a fresh context.
    try {
      Object.keys(pool).forEach(function (k) {
        try {
          hardLoseOne(pool[k]);
        } catch (eP) {}
        delete pool[k];
      });
    } catch (ePool) {}
    try {
      global.__GR_LAST_GL_HARD_LOSE__ = {
        at: Date.now(),
        why: String(why || "retry"),
        forced_lose: stats.forced_lose,
      };
    } catch (eMeta) {}
    // Restore soft-only policy so normal multipath does not thrash Firefox.
    allowHardLose = prev;
    return stats.forced_lose;
  }

  global.GRGlGovernor = {
    releaseAll: softReleaseAll,
    releaseSoft: softReleaseAll,
    releaseHard: hardReleaseAll,
    /** Hang recovery only — see forceHardLoseForRetry. */
    forceHardLoseForRetry: forceHardLoseForRetry,
    forceCap: function () {
      // Idle hard free of excess only
      forceCap();
      scheduleHardLose();
    },
    liveCount: function () {
      pruneDead();
      return live.length;
    },
    stats: function () {
      pruneDead();
      return Object.assign({}, stats, { live: live.length, max: MAX_LIVE, pool: Object.keys(pool) });
    },
    /** Product fields: governed renderer isolation (iss/46 H3). */
    snapshotFields: snapshotFields,
    setMaxLive: function (n) {
      MAX_LIVE = Math.max(1, Math.min(2, n | 0));
      forceCap();
    },
    /**
     * Max residual multipath plans per B10 run (serial create+release).
     * Commercial near-silicon needs ≥3 (float + noderiv/rint + ulp).
     * weak: 2; healthy desktop: 4; never >5.
     *
     * Prod root-cause (v5.8.120 mirror): when live.length>=MAX_LIVE this returned 1,
     * collapsing 759/808 B10 sessions to a single residual path → res/wg class floor.
     * Soft-release first; never starve commercial multipath just because slots are full.
     */
    pathCap: function (weak) {
      try {
        pruneDead();
        if (live.length >= MAX_LIVE) {
          try {
            softReleaseAll();
          } catch (eRel) {}
          pruneDead();
        }
      } catch (e) {}
      if (weak) return 2;
      return 4;
    },
    /** Prefer for collectors: shared canvas+gl. */
    acquire: function (size, type) {
      var t = type || "webgl";
      var c = document.createElement("canvas");
      var s = size || 256;
      c.width = s;
      c.height = s;
      var gl = c.getContext(t) || c.getContext("webgl") || c.getContext("experimental-webgl");
      return gl ? { gl: gl, canvas: c } : null;
    },
    /** Single-flight WebGPU adapter (muted console). */
    requestWebGpuAdapter: requestWebGpuAdapterSafe,
  };
  try {
    global.__GR_GL_GOV_SNAPSHOT__ = snapshotFields;
  } catch (eSnap) {}

  try {
    global.__GR_GL_GOV_RELEASE__ = softReleaseAll;
  } catch (e2) {}
})(typeof window !== "undefined" ? window : globalThis);
