/**
 * GR version-path loader — inject should pin:
 *   /g5/dist/v/<VERSION>/gr.loader.min.js
 * Physical file is still flat fe/gr.loader.min.js (server strip_version_route).
 * 1) Read inject boot config
 * 2) GET /v1/sdk/bootstrap (no-store) for authoritative product_version + asset URLs
 * 3) Load micro/entry under /g5/dist/v/<VERSION>/…
 *
 * Build stamps window.__GR_BUILD_IMPL__ so uploads can report fe_impl_version.
 */
(function () {
  "use strict";
  try {
    if (window.__GR_LOADER_RAN__) return;
    window.__GR_LOADER_RAN__ = 1;

    var boot = window.__GR_BOOT__ || {};
    var P = String(boot.apiBase || boot.first_party_path || "/g5").replace(/\/$/, "") || "/g5";
    var injectV = String(boot.version || boot.product_version || boot.sdk_v || "");
    var siteId = boot.site_id || boot.siteId || window.__GR_SITE_ID__ || "";
    var gw =
      boot.gwBase ||
      window.__GR_GW_DIRECT__ ||
      boot.gw_base ||
      "";
    // Record loader implementation identity ASAP (build stamp or inject V).
    try {
      var loaderImpl =
        (typeof window.__GR_BUILD_IMPL__ !== "undefined" && window.__GR_BUILD_IMPL__) ||
        injectV ||
        "";
      if (loaderImpl) {
        window.__GR_BUILD_IMPL__ = window.__GR_BUILD_IMPL__ || loaderImpl;
        window.__GR_FE_IMPL__ = window.__GR_FE_IMPL__ || {};
        window.__GR_FE_IMPL__.loader = String(loaderImpl);
        window.__GR_FE_IMPL__.build = window.__GR_FE_IMPL__.build || String(loaderImpl);
        window.__GR_FE_LOADER_IMPL__ = String(loaderImpl);
      }
      try {
        var cur =
          (document.currentScript && document.currentScript.src) ||
          (boot.loader_url || "");
        if (cur) {
          window.__GR_FE_IMPL__ = window.__GR_FE_IMPL__ || {};
          window.__GR_FE_IMPL__.loader_url = String(cur).slice(0, 240);
        }
      } catch (eUrl) {}
    } catch (eImpl) {}

    function loadScript(src, asyncAttr, attrs) {
      return new Promise(function (resolve, reject) {
        try {
          if (document.querySelector('script[src="' + src + '"]')) {
            resolve(src);
            return;
          }
          var s = document.createElement("script");
          s.src = src;
          s.async = !!asyncAttr;
          if (attrs) {
            Object.keys(attrs).forEach(function (k) {
              if (attrs[k] != null) s.setAttribute(k, String(attrs[k]));
            });
          }
          s.onload = function () {
            resolve(src);
          };
          s.onerror = function () {
            reject(new Error("load_fail " + src));
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (e) {
          reject(e);
        }
      });
    }

    function applyVersion(V, extra) {
      V = String(V || injectV || "");
      extra = extra || {};
      var gen =
        (extra.bootstrap && extra.bootstrap.asset_gen) ||
        extra.asset_gen ||
        (boot && boot.asset_gen) ||
        "";
      // SSOT for ALL subsequent asset version paths.
      window.__GR_SERVER_PRODUCT_VERSION__ = V;
      window.__GR_PRODUCT_VERSION__ = V;
      if (gen) window.__GR_ASSET_GEN__ = String(gen);
      try {
        window.__GR_FE_IMPL__ = window.__GR_FE_IMPL__ || {};
        window.__GR_FE_IMPL__.product_version = V;
        if (gen) window.__GR_FE_IMPL__.asset_gen = String(gen);
        if (extra.bootstrap && extra.bootstrap.fe_impl_version) {
          window.__GR_FE_IMPL__.server_fe_impl = String(extra.bootstrap.fe_impl_version);
        }
      } catch (eFi) {}
      window.__GR_BOOT__ = Object.assign({}, boot, extra || {}, {
        version: V,
        product_version: V,
        asset_gen: gen || undefined,
        apiBase: P,
        first_party_path: P,
        site_id: siteId || boot.site_id,
        siteId: siteId || boot.siteId,
        gwBase: gw || boot.gwBase,
      });

      try {
        var prev = sessionStorage.getItem("gr_fe_adopted_v") || "";
        if (prev && prev !== V) {
          sessionStorage.removeItem("gr_hot_swap_ok_" + prev);
          sessionStorage.removeItem("gr_upgrade_once_" + prev);
        }
        if (V) sessionStorage.setItem("gr_fe_adopted_v", V);
      } catch (eSs) {}
      return V;
    }

    /** 1.0.3+: version-keyed dist root — /dist/v/<V>/ (cache rotates per release). */
    function distV(_V) {
      var v = String(_V || window.__GR_PRODUCT_VERSION__ || injectV || "");
      if (v) return P + "/dist/v/" + v + "/";
      return P + "/dist/";
    }

    function withGenLeaf(name) {
      var gen = "";
      try {
        gen = String(
          (window.__GR_MANIFEST__ && window.__GR_MANIFEST__.asset_gen) ||
            window.__GR_ASSET_GEN__ ||
            ""
        );
      } catch (eG) {}
      name = String(name || "");
      var base = name.replace(/^.*\//, "");
      // pure opaque or embedded content-hash — never double-inject gen
      if (
        /^[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base) ||
        /(?:\.|\/)[a-f0-9]{8,16}\.(min\.)?(js|wasm|html|css)$/i.test(base)
      ) {
        return name;
      }
      // Opaque-only: do not emit meaningful product basenames on the wire.
      if (
        /(?:^|[\/.])(gr\.|registry\.|pack_loader|gl_governor|probe_self_heal|nest_frame)/i.test(
          base
        )
      ) {
        return "";
      }
      if (!gen) return name;
      if (/\.min\.js$/i.test(name)) return name.replace(/\.min\.js$/i, "." + gen + ".min.js");
      if (/\.js$/i.test(name)) return name.replace(/\.js$/i, "." + gen + ".js");
      return name;
    }

    function loadGlGovernor(V, explicitUrl) {
      // Prefer bootstrap gl_governor_url when present (already content-hashed / opaque).
      if (explicitUrl) {
        return loadScript(explicitUrl, false).catch(function () {
          return "gl_gov_skip";
        });
      }
      try {
        var man = window.__GR_MANIFEST__ || null;
        var fromMan =
          (man && man.assets && man.assets.gl_governor) ||
          (man && man.gl_governor_url) ||
          "";
        if (fromMan) {
          return loadScript(String(fromMan), false).catch(function () {
            return "gl_gov_skip";
          });
        }
      } catch (eM) {}
      // Opaque-only: do not invent meaningful `gr.gl_governor` filenames on the wire.
      return Promise.resolve("gl_gov_skip");
    }

    function loadAgents(V) {
      V = applyVersion(V);
      var base = distV(V);
      // Opaque-only: require manifest/bootstrap hashed URLs — never invent product basenames.
      var microLeaf = withGenLeaf("gr.micro.min.js");
      var entryLeaf = withGenLeaf("gr.entry.min.js");
      if (!microLeaf || !entryLeaf) {
        return Promise.reject(new Error("opaque_only_no_logical_fallback"));
      }
      var micro = base + microLeaf;
      var entry = base + entryLeaf;
      // micro SYNC-style: async=false so it runs open before entry races
      return loadGlGovernor(V)
        .then(function () {
          return loadScript(micro, false);
        })
        .then(function () {
          return loadScript(entry, true, {
            "data-inject-path": "nginx",
            "data-endpoint": P,
            "data-gw-base": gw || "",
            "data-site-id": siteId || "",
          });
        })
        .catch(function (err) {
          try {
            if (window.GROps && GROps.report) {
              GROps.report(
                "loader_script_fail",
                "loader",
                { err: String(err && err.message ? err.message : err).slice(0, 120), v: V },
                "error"
              );
            }
          } catch (eO) {}
          // last resort: entry only (governor already attempted)
          return loadScript(entry, true, {
            "data-inject-path": "nginx",
            "data-endpoint": P,
            "data-site-id": siteId || "",
          });
        });
    }

    // Bootstrap first (authoritative version). Fallback to inject V if offline.
    var ctrl = typeof AbortController !== "undefined" ? new AbortController() : null;
    var to = setTimeout(function () {
      try {
        if (ctrl) ctrl.abort();
      } catch (eA) {}
    }, 2500);

    fetch(P + "/v1/sdk/bootstrap", {
      method: "GET",
      credentials: "same-origin",
      cache: "no-store",
      signal: ctrl ? ctrl.signal : undefined,
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        clearTimeout(to);
        var V = (j && (j.product_version || j.version)) || injectV;
        // Standard C: apply full bootstrap as manifest so entry/boot resolve
        // opaque assets (rpa_monitor, seal wasm, pack_tokens) instead of
        // inventing logical.<asset_gen> content-hash fallbacks.
        try {
          if (j && typeof j === "object") {
            window.__GR_MANIFEST__ = j;
            if (j.asset_gen) window.__GR_ASSET_GEN__ = String(j.asset_gen);
            if (j.seal_v2) {
              boot = Object.assign({}, boot, { seal_v2: j.seal_v2, manifest: j });
            } else {
              boot = Object.assign({}, boot, { manifest: j });
            }
          }
        } catch (eMan) {}
        if (j && j.entry_url && j.micro_url) {
          applyVersion(V, { bootstrap: j, manifest: j, seal_v2: j.seal_v2 });
          var microUrl = j.micro_url;
          var entryUrl = j.entry_url;
          var glUrl =
            (j.gl_governor_url ||
              (j.assets && j.assets.gl_governor) ||
              "") + "";
          // Prefer bootstrap URLs (opaque / content-hashed under /g5/dist/).
          if (microUrl.indexOf("/g5/") === 0 || microUrl.indexOf("/dist/") >= 0) {
            return loadGlGovernor(V, glUrl || null)
              .then(function () {
                return loadScript(microUrl, false);
              })
              .then(function () {
                return loadScript(entryUrl, true, {
                  "data-inject-path": "nginx",
                  "data-endpoint": P,
                  "data-gw-base": gw || "",
                  "data-site-id": siteId || "",
                });
              });
          }
        }
        return loadAgents(V);
      })
      .catch(function () {
        clearTimeout(to);
        return loadAgents(injectV);
      });
  } catch (e) {
    try {
      var b = window.__GR_BOOT__ || {};
      var base = String(b.apiBase || "/g5").replace(/\/$/, "") || "/g5";
      var gen = "";
      try {
        gen = String(window.__GR_ASSET_GEN__ || (b.asset_gen || "") || "");
      } catch (eG) {}
      var s = document.createElement("script");
      // 1.0.3+: version-keyed path (no ?v= query busting); gen in filename when known.
      var fv = String(window.__GR_PRODUCT_VERSION__ || injectV || "");
      s.src =
        base +
        (fv ? "/dist/v/" + fv + "/" : "/dist/") +
        (gen ? "gr.entry." + gen + ".min.js" : "gr.entry.min.js");
      s.async = true;
      (document.head || document.documentElement).appendChild(s);
    } catch (e2) {}
  }
})();
