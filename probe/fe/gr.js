/**
 * GR stable pin (protocol 2) — eternal inject URL:
 *   /g5/gr.js
 *
 * Responsibilities ONLY:
 *  1) Queue (dataLayer-style): window.gr / __GR_Q__
 *  2) GET /v1/sdk/bootstrap (no-store) → full manifest
 *  3) Load scripts by wave (parallel within wave; deps across waves)
 *  4) Stamp product_version + asset_base for pack_loader / brain
 *
 * No probe logic. Body should change rarely; VERSION lives in bootstrap.
 */
(function (w, d) {
  "use strict";
  try {
    if (w.__GR_PIN_RAN__) return;
    w.__GR_PIN_RAN__ = 1;
  } catch (e0) {
    return;
  }

  w.__GR_Q__ = w.__GR_Q__ || [];
  if (typeof w.gr !== "function") {
    w.gr = function () {
      try {
        w.__GR_Q__.push(arguments);
      } catch (eQ) {}
    };
  }

  var boot = w.__GR_BOOT__ || {};
  // API/upload base (often https://gv.*) — NOT for static pack load under first_party.
  var P =
    String(boot.apiBase || boot.first_party_path || "/g5").replace(/\/$/, "") ||
    "/g5";
  // Static asset root: first_party nginx /g5 (page origin). Never join with gv host.
  var A =
    String(
      boot.assetBase ||
        boot.asset_base ||
        boot.first_party_path ||
        (w.__GR_FIRST_PARTY__ ? "/g5" : "") ||
        "/g5"
    ).replace(/\/$/, "") || "/g5";
  var siteId = boot.site_id || boot.siteId || w.__GR_SITE_ID__ || "";
  var gw = boot.gwBase || boot.gw_base || w.__GR_GW_DIRECT__ || "";

  var loadedOk = Object.create(null);
  function loadScript(src, asyncAttr, attrs) {
    return new Promise(function (resolve, reject) {
      try {
        if (!src) {
          resolve("");
          return;
        }
        if (loadedOk[src]) {
          resolve(src);
          return;
        }
        var s = d.createElement("script");
        var url = src;
        // A previous failed <script src> still matches querySelector — do not
        // treat DOM presence as success (retry must re-fetch).
        if (d.querySelector('script[src="' + src + '"]')) {
          url = src + (src.indexOf("?") >= 0 ? "&" : "?") + "gr_pin_r=" + Date.now();
        }
        s.src = url;
        s.async = !!asyncAttr;
        if (attrs) {
          Object.keys(attrs).forEach(function (k) {
            if (attrs[k] != null) s.setAttribute(k, String(attrs[k]));
          });
        }
        s.onload = function () {
          loadedOk[src] = 1;
          resolve(src);
        };
        s.onerror = function () {
          reject(new Error("load_fail " + src));
        };
        (d.head || d.documentElement).appendChild(s);
      } catch (e) {
        reject(e);
      }
    });
  }

  function preload(url) {
    try {
      if (!url || !d.head) return;
      if (d.querySelector('link[rel="preload"][href="' + url + '"]')) return;
      var l = d.createElement("link");
      l.rel = "preload";
      l.as = "script";
      l.href = url;
      d.head.appendChild(l);
    } catch (eP) {}
  }

  /**
   * Resolve static asset URLs (packs / seal wasm) for two load modes:
   * - first_party: page-relative /g5/dist/... (www nginx; NEVER join apiBase=https://gv)
   * - pv: https://pv.host/dist/... (pv serves /dist, NOT /g5/dist)
   * Upload/open still uses P (apiBase), which may be https://gv.
   */
  function resolveAssetUrl(pathOrUrl) {
    var u = String(pathOrUrl || "");
    if (!u) return "";
    var assetRoot = String(A || "/g5").replace(/\/$/, "") || "/g5";
    var feLoad = String(boot.fe_load || boot.feLoad || (w.__GR_FIRST_PARTY__ ? "first_party" : "") || "").toLowerCase();
    var pvMode =
      feLoad === "pv" ||
      /^https?:\/\/pv\./i.test(assetRoot) ||
      (boot.script_base && /^https?:\/\/pv\./i.test(String(boot.script_base)));

    // Normalize path: strip accidental gv host; map /g5/dist → /dist for pv hosts
    function stripGvG5(absUrl) {
      try {
        var abs = new URL(absUrl);
        if (/^gv\./i.test(abs.hostname)) {
          // Never load static from gv with /g5 prefix (JSON 404). Prefer assetRoot.
          var path = abs.pathname || "/";
          if (path.indexOf("/g5/") === 0 || path === "/g5") {
            path = path.replace(/^\/g5(?=\/|$)/, "") || "/";
          }
          if (pvMode || /^https?:\/\//i.test(assetRoot)) {
            return resolveAssetUrl(path);
          }
          // first_party: force page-relative /g5...
          if (path.indexOf("/dist") === 0) return "/g5" + path;
          return path.charAt(0) === "/" ? path : "/" + path;
        }
        // absolute pv/cdn already correct
        return abs.href;
      } catch (eA) {
        return absUrl;
      }
    }

    if (/^https?:\/\//i.test(u)) {
      return stripGvG5(u);
    }

    // Root-absolute path
    if (u.charAt(0) === "/") {
      if (pvMode || /^https?:\/\//i.test(assetRoot)) {
        // pv host: /g5/dist/x → /dist/x under https://pv
        var pathPv = u;
        if (pathPv.indexOf("/g5/") === 0 || pathPv === "/g5") {
          pathPv = pathPv.replace(/^\/g5(?=\/|$)/, "") || "/";
        }
        if (pathPv.indexOf("/dist") !== 0 && pathPv !== "/gr.js" && pathPv.indexOf("/gr.") !== 0) {
          // leave other absolute paths
        }
        try {
          return new URL(pathPv, assetRoot.charAt(assetRoot.length - 1) === "/" ? assetRoot : assetRoot + "/").href;
        } catch (eU) {
          return assetRoot.replace(/\/$/, "") + (pathPv.charAt(0) === "/" ? pathPv : "/" + pathPv);
        }
      }
      // first_party: keep /g5/... page-relative (document origin = www)
      return u;
    }
    // Relative fragment
    if (/^https?:\/\//i.test(assetRoot)) {
      return assetRoot.replace(/\/$/, "") + "/" + u.replace(/^\//, "");
    }
    return assetRoot + "/" + u.replace(/^\//, "");
  }

  /** Rewrite seal_v2 wasm/loader paths to page-relative /g5 (never gv+/g5). */
  function rewriteSealMeta(j) {
    if (!j || !j.seal_v2 || typeof j.seal_v2 !== "object") return j;
    try {
      var s = j.seal_v2;
      ["wasm_url", "wasm_url_flat", "loader_url", "loader_url_flat"].forEach(function (k) {
        if (s[k]) s[k] = resolveAssetUrl(s[k]);
      });
    } catch (eS) {}
    return j;
  }

  function applyManifest(j) {
    // Prefer FE content version for asset paths; keep product_version as runtime stamp.
    var runtimeV = String((j && (j.product_version || j.version)) || "") || "";
    var feV = String((j && (j.fe_version || j.fe_impl_version)) || runtimeV || "") || "";
    var V = feV || runtimeV;
    var gen = String((j && j.asset_gen) || "") || "";
    // 1.0.3+: asset_base is version-keyed (/dist/v/<fe>/g/<gen>/) — keep the
    // version/gen segments verbatim; they rotate caches across releases.
    var base = String((j && j.asset_base) || A + "/dist");
    base = resolveAssetUrl(base);
    // Ensure versioned packs stay under /g5 when bootstrap sent absolute gv paths incorrectly
    if (/^https?:\/\//i.test(base) === false && base.charAt(0) !== "/") {
      base = A + "/" + base.replace(/^\//, "");
    }
    j = rewriteSealMeta(j);
    try {
      w.__GR_SERVER_PRODUCT_VERSION__ = runtimeV || V;
      w.__GR_PRODUCT_VERSION__ = V;
      w.__GR_FE_VERSION__ = feV || V;
      w.__GR_ASSET_BASE__ = base;
      w.__GR_MANIFEST__ = j;
      w.__GR_BOOTSTRAP_PROTOCOL__ = (j && j.protocol) || 2;
      if (gen) w.__GR_ASSET_GEN__ = gen;
      if (j && j.asset_gen) w.__GR_ASSET_GEN__ = String(j.asset_gen);
      if (j && j.fe_impl_version) {
        w.__GR_FE_IMPL__ = w.__GR_FE_IMPL__ || {};
        w.__GR_FE_IMPL__.server_fe_impl = String(j.fe_impl_version);
        w.__GR_FE_IMPL__.product_version = runtimeV || V;
        w.__GR_FE_IMPL__.fe_version = feV || V;
      }
      try {
        if (gen) sessionStorage.setItem("gr_asset_gen", gen);
        if (feV) sessionStorage.setItem("gr_fe_version", feV);
      } catch (eSsGen) {}
      var bh = (j && j.brain_hints) || {};
      if (bh.upload_concurrency != null) w.__GR_UPLOAD_CONCURRENCY__ = Number(bh.upload_concurrency);
      if (bh.mid_ramp_concurrency != null) w.__GR_MID_RAMP_CONCURRENCY__ = Number(bh.mid_ramp_concurrency);
      if (bh.max_light_concurrent != null) w.__GR_PACK_MAX_LIGHT__ = Number(bh.max_light_concurrent);
    } catch (eM) {}

    w.__GR_BOOT__ = Object.assign({}, boot, {
      version: V,
      product_version: V,
      assetBase: A,
      asset_base: base,
      apiBase: P,
      // first_party_path is the LOAD path (/g5), not the upload API host
      first_party_path: A.indexOf("http") === 0 ? "/g5" : A || "/g5",
      site_id: siteId || boot.site_id,
      siteId: siteId || boot.siteId,
      gwBase: gw || boot.gwBase,
      require_sealed_ingest:
        j && j.require_sealed_ingest != null
          ? !!j.require_sealed_ingest
          : boot.require_sealed_ingest,
      manifest: j,
      seal_v2: (j && j.seal_v2) || boot.seal_v2,
    });

    try {
      if (V) sessionStorage.setItem("gr_fe_adopted_v", V);
    } catch (eSs) {}
    return { V: V, base: base };
  }

  function normalizeScripts(j) {
    function one(id, url, wave, deps) {
      return {
        id: id,
        url: resolveAssetUrl(url),
        wave: wave,
        async: true,
        deps: deps || [],
      };
    }
    if (j && Array.isArray(j.scripts) && j.scripts.length) {
      return j.scripts.map(function (s, i) {
        if (typeof s === "string") {
          return one("s" + i, s, i === 0 ? 0 : 1);
        }
        return {
          id: s.id || "s" + i,
          url: resolveAssetUrl(s.url || s.src || ""),
          wave: s.wave != null ? Number(s.wave) : 1,
          async: s.async !== false,
          deps: s.deps || [],
          priority: s.priority || "auto",
          blocking: !!s.blocking,
        };
      });
    }
    // protocol 1 fallback fields
    var list = [];
    if (j && j.gl_governor_url) {
      list.push(one("gl_governor", j.gl_governor_url, 0));
    }
    if (j && j.micro_url) {
      list.push(one("micro", j.micro_url, 0));
    }
    if (j && j.entry_url) {
      list.push(one("entry", j.entry_url, 1, ["micro"]));
    }
    var assets = (j && j.assets) || {};
    // entry already inlines pack_loader — avoid duplicate script fetch on pin path.
    if (assets.lite) {
      list.push(one("lite", assets.lite, 1, ["entry"]));
    }
    return list;
  }

  function loadByWaves(scripts) {
    var byWave = Object.create(null);
    var maxW = 0;
    scripts.forEach(function (s) {
      if (!s.url) return;
      var wnum = s.wave != null ? s.wave : 1;
      if (wnum > maxW) maxW = wnum;
      if (!byWave[wnum]) byWave[wnum] = [];
      byWave[wnum].push(s);
    });
    // Preload all known URLs early (parallel download, ordered exec per wave)
    scripts.forEach(function (s) {
      if (s.url) preload(s.url);
    });
    var layers = null;
    try {
      layers = w.__GR_MANIFEST__ && w.__GR_MANIFEST__.layers;
    } catch (eL) {}
    if (layers && layers.hard) preload(layers.hard);

    var chain = Promise.resolve();
    var wi;
    for (wi = 0; wi <= maxW; wi++) {
      (function (wave, items) {
        if (!items || !items.length) return;
        chain = chain.then(function () {
          // same wave: parallel
          return Promise.all(
            items.map(function (s) {
              var asyncAttr = s.async !== false && !s.blocking;
              var attrs = {};
              if (s.id === "entry" || s.id === "micro") {
                attrs["data-inject-path"] = "nginx";
                attrs["data-endpoint"] = P;
                if (gw) attrs["data-gw-base"] = gw;
                if (siteId) attrs["data-site-id"] = siteId;
              }
              if (s.id === "entry") attrs.fetchpriority = "high";
              return loadScript(s.url, asyncAttr, attrs).catch(function (err) {
                try {
                  w.__GR_PIN_LOAD_ERR__ = w.__GR_PIN_LOAD_ERR__ || [];
                  w.__GR_PIN_LOAD_ERR__.push(String(err && err.message ? err.message : err));
                } catch (eE) {}
                return null;
              });
            })
          );
        });
      })(wi, byWave[wi]);
    }
    return chain;
  }

  function fetchBootstrapFrom(base) {
    var root = String(base || "/g5").replace(/\/$/, "") || "/g5";
    var url = root + "/v1/sdk/bootstrap";
    if (siteId) {
      url += (url.indexOf("?") >= 0 ? "&" : "?") + "site_id=" + encodeURIComponent(siteId);
    }
    var ctrl = typeof AbortController !== "undefined" ? new AbortController() : null;
    var to = setTimeout(function () {
      try {
        if (ctrl) ctrl.abort();
      } catch (eA) {}
    }, 8000);
    // Cross-origin gv bootstrap: omit credentials (public manifest). same-origin only when relative /g5.
    var cred = /^https?:\/\//i.test(root) ? "omit" : "same-origin";
    return fetch(url, {
      method: "GET",
      credentials: cred,
      mode: "cors",
      cache: "no-store",
      signal: ctrl ? ctrl.signal : undefined,
      headers: { Accept: "application/json" },
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        clearTimeout(to);
        return j;
      })
      .catch(function (e) {
        clearTimeout(to);
        throw e;
      });
  }

  function fetchBootstrap() {
    return fetchBootstrapFrom(P).catch(function (e0) {
      // Dual/third-party apiBase (gv.*) can be blocked or slow; first-party /g5
      // is always proxied on the business host in lab/prod nginx inject.
      if (String(P) !== "/g5" && String(P).indexOf("/g5") !== 0) {
        try {
          w.__GR_PIN_BOOTSTRAP_FP__ = String(e0 && e0.message ? e0.message : e0).slice(0, 160);
        } catch (eN) {}
        return fetchBootstrapFrom("/g5");
      }
      throw e0;
    });
  }

  function bootLive() {
    try {
      return !!(
        w.__GR_BOOT_STARTED__ ||
        w.__GR_SESSION_ID__ ||
        (w.GRBoot && w.GRBoot.start)
      );
    } catch (e) {
      return false;
    }
  }

  function pageUnloading() {
    try {
      return !!w.__GR_PAGE_UNLOADING__;
    } catch (e) {
      return false;
    }
  }

  function markPinReady() {
    try {
      w.__GR_PIN_READY__ = 1;
      w.__GR_PIN_FAIL__ = "";
      w.dispatchEvent(new Event("gr-pin-ready"));
    } catch (eR) {}
  }

  function run() {
    return fetchBootstrap()
      .catch(function () {
        return new Promise(function (res, rej) {
          setTimeout(function () {
            fetchBootstrap().then(res, rej);
          }, 400);
        });
      })
      .then(function (j) {
        if (!j || j.ok === false) throw new Error("bootstrap_fail");
        applyManifest(j);
        var scripts = normalizeScripts(j);
        if (!scripts.length) throw new Error("no_scripts");
        return loadByWaves(scripts).then(function () {
          if (!bootLive()) throw new Error("boot_missing_after_waves");
          markPinReady();
        });
      })
      .catch(function (err) {
        try {
          w.__GR_PIN_FAIL__ = String(err && err.message ? err.message : err);
        } catch (eF) {}
        // Last-resort Standard C: hashed loader under flat /dist/ only (no version path).
        try {
          var legacy = String(boot.loader_url_legacy || "");
          if (!legacy || !/(?:\.|\/)[a-f0-9]{8,16}\.(min\.)?js(\?|#|$)/i.test(legacy)) {
            var gen = "";
            try {
              gen = sessionStorage.getItem("gr_asset_gen") || "";
            } catch (eSs) {}
            if (gen) {
              legacy = A + "/dist/gr.loader." + gen + ".min.js";
            } else if (String(A) !== "/g5") {
              // Cross-origin asset host failed — try first-party hashed loader later via watchdog.
              try {
                w.__GR_PIN_FAIL__ =
                  String(w.__GR_PIN_FAIL__ || err) + "|no_hashed_loader_fallback";
              } catch (eN) {}
              return null;
            } else {
              try {
                w.__GR_PIN_FAIL__ =
                  String(w.__GR_PIN_FAIL__ || err) + "|no_hashed_loader_fallback";
              } catch (eN2) {}
              return null;
            }
          }
          // 1.0.3+: keep /dist/v/<ver>/g/<gen>/ segments in the loader fallback
          // URL (version-keyed paths rotate caches; strip_version_route on the
          // plane resolves the logical file).
          return loadScript(resolveAssetUrl(String(legacy)), true).then(function () {
            if (bootLive()) markPinReady();
            return null;
          });
        } catch (eL) {
          return null;
        }
      });
  }

  var runLock = false;
  var watchN = 0;
  function runSafe() {
    if (runLock || bootLive() || pageUnloading()) return Promise.resolve();
    runLock = true;
    return run()
      .then(function () {
        runLock = false;
      })
      .catch(function () {
        runLock = false;
      });
  }
  function scheduleWatch() {
    if (bootLive() || pageUnloading()) return;
    watchN += 1;
    if (watchN > 14) return;
    var delay = Math.min(20000, 1200 * watchN);
    setTimeout(function () {
      if (bootLive() || pageUnloading()) return;
      runSafe().then(function () {
        if (!bootLive()) scheduleWatch();
      });
    }, delay);
  }
  runSafe().then(function () {
    if (!bootLive()) scheduleWatch();
  });
})(window, document);
