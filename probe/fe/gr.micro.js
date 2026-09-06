/**
 * GR micro-kick — inline-early open∥B8 (production nginx inject companion).
 *
 * Loaded as first script before gr.entry.min.js. product_version is authority:
 * - same V + cool → skip identity (cool open only)
 * - version change / no cool → remint cycle stamped with V, fire open + B8
 * Never requires the user to clear cookies.
 */
(function () {
  "use strict";
  try {
    var boot = (typeof window !== "undefined" && window.__GR_BOOT__) || {};
    var S = boot.site_id || boot.siteId || window.__GR_SITE_ID__ || "";
    // apiBase = upload host (prod: https://gv). No /g5-gw fallback — not a real failover.
    var P = String(boot.apiBase || boot.gwBase || window.__GR_GW_DIRECT__ || "").replace(
      /\/$/,
      ""
    );
    var G = String(
      boot.gwBase || window.__GR_GW_DIRECT__ || boot.gw_base || P || ""
    ).replace(/\/$/, "");
    if (!P) P = G;
    if (!G) G = P;
    var V = String(boot.version || boot.product_version || boot.sdk_v || "");
    if (!V && typeof document !== "undefined") {
      try {
        var sc = document.currentScript && document.currentScript.src;
        var m = sc && sc.match(/[?&]v=([^&]+)/);
        if (m) V = decodeURIComponent(m[1]);
      } catch (eV0) {}
    }
    // Lab/ops: ?gr_force=1 forces re-probe (clears local cool) without manual cookie wipe.
    var forceIdentity = false;
    try {
      forceIdentity =
        !!(window.__GR_FORCE_IDENTITY__ || boot.force_identity) ||
        /(?:^|[?&])gr_force=1(?:&|$)/.test(String(location.search || ""));
      if (forceIdentity) window.__GR_FORCE_IDENTITY__ = 1;
    } catch (eF) {}

    function pdDomain() {
      try {
        var d = String(location.hostname || "");
        // IP / localhost / reserved suffixes: host-only (never Domain=.gr.local).
        if (!d || d === "localhost" || /^\d+\.\d+\.\d+\.\d+$/.test(d) || d.indexOf(":") >= 0) {
          return "";
        }
        var ps = d.split(".");
        var last = String(ps[ps.length - 1] || "").toLowerCase();
        if (
          {
            local: 1,
            test: 1,
            invalid: 1,
            localhost: 1,
            internal: 1,
            lan: 1,
            home: 1,
            corp: 1,
            localdomain: 1,
          }[last]
        ) {
          return "";
        }
        if (
          ps.length >= 3 &&
          ["com", "net", "org", "edu", "co"].indexOf(String(ps[ps.length - 2]).toLowerCase()) >= 0
        ) {
          return "." + ps.slice(-3).join(".");
        }
        if (ps.length >= 2) return "." + ps.slice(-2).join(".");
      } catch (e) {}
      return "";
    }
    var pd = pdDomain();
    function embedTokenFromUrl() {
      try {
        if (boot.embed_token) return String(boot.embed_token);
        // 同 gr.boot.js：currentScript.src 可能是无 grt 的资产 URL(真值)，
        // 不能短路 — 依次尝试 currentScript/pin_url/含 grt= 的 script 标签。
        var srcs = [];
        try {
          if (document.currentScript && document.currentScript.src)
            srcs.push(String(document.currentScript.src));
        } catch (eCs) {}
        if (boot.pin_url) srcs.push(String(boot.pin_url));
        try {
          if (document.querySelectorAll) {
            var tags = document.querySelectorAll('script[src*="grt="]');
            for (var i = 0; i < tags.length; i++) srcs.push(String(tags[i].src || ""));
          }
        } catch (eQs) {}
        for (var j = 0; j < srcs.length; j++) {
          var m = srcs[j].match(/[?&]grt=([^&]+)/);
          if (m) return decodeURIComponent(m[1]);
        }
      } catch (eTok) {}
      return "";
    }
    function collectCookieFields() {
      var names = [];
      try {
        var raw = boot.cookie_fields || window.__GR_COOKIE_FIELDS__ || [];
        if (typeof raw === "string") raw = raw.split(",");
        if (Array.isArray(raw)) names = raw;
      } catch (eN) {}
      var out = {};
      if (!names.length) return out;
      var jar = "";
      try {
        jar = String(document.cookie || "");
      } catch (eJ) {
        return out;
      }
      for (var i = 0; i < names.length && i < 16; i++) {
        var n = String(names[i] || "").trim();
        if (!n || n.length > 64 || !/^[a-zA-Z0-9_.-]+$/.test(n)) continue;
        var re = new RegExp(
          "(?:^|;\\s*)" + n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "=([^;]*)"
        );
        var m = jar.match(re);
        if (m && m[1]) {
          try {
            out[n] = decodeURIComponent(m[1]).slice(0, 256);
          } catch (eD) {
            out[n] = String(m[1]).slice(0, 256);
          }
        }
      }
      return out;
    }
    var microCookies = collectCookieFields();
    var microTok = embedTokenFromUrl();

    function ck(n, v, a) {
      var b =
        n +
        "=" +
        encodeURIComponent(v == null ? "" : String(v)) +
        ";path=/;max-age=" +
        (a || 86400) +
        ";samesite=lax" +
        (location.protocol === "https:" ? ";secure" : "");
      document.cookie = pd ? b + ";domain=" + pd : b;
      document.cookie = b;
    }
    function expireCk(n) {
      var secure = location.protocol === "https:" ? ";secure" : "";
      document.cookie = n + "=;path=/;max-age=0;samesite=lax" + secure;
      try {
        var h = String(location.hostname || "");
        var parts = h.split(".");
        for (var i = 0; i < parts.length - 1; i++) {
          document.cookie =
            n + "=;path=/;max-age=0;samesite=lax;domain=." + parts.slice(i).join(".") + secure;
        }
      } catch (eX) {}
    }
    function siteTok() {
      try {
        var boot = window.__GR_BOOT__ || {};
        var s = boot.site_id || boot.siteId || window.__GR_SITE_ID__ || "";
        return String(s).replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 48);
      } catch (e) {
        return "";
      }
    }
    function cycleCkName(base) {
      var s = siteTok();
      return s ? base + "." + s : base;
    }
    function readCk(name) {
      try {
        var parts = document.cookie.split(";");
        for (var i = 0; i < parts.length; i++) {
          var p = parts[i].replace(/^\s+/, "");
          if (p.indexOf(name + "=") === 0) return decodeURIComponent(p.slice(name.length + 1));
        }
      } catch (e) {}
      return "";
    }

    // Prefer GRStorage when entry/storage already present (rare on first paint).
    if (window.GRStorage && typeof window.GRStorage.ensureProbeStateForVersion === "function") {
      var st0 = window.GRStorage.ensureProbeStateForVersion(V);
      window.__GR_CYCLE_HINT__ = st0.cycleId;
      window.__GR_SESSION_ID__ = st0.cycleId;
      window.__GR_MICRO_KICK__ = 1;
      if (st0.coolActive) {
        window.__GR_PHASE__ = "cool";
        window.__GR_STOP_PROBE__ = true;
        window.__GR_SKIP_IDENTITY__ = true;
        window.__GR_HALT_UPLOADS__ = true;
      }
    }

    // Read primary (_g5_*) first, then legacy gr_* (must match storage.js dual-write).
    function readCoolUntil() {
      var n = parseInt(readCk("_g5_k") || readCk("gr_cool_until_v1") || "0", 10) || 0;
      if (!n) {
        try {
          n =
            parseInt(
              localStorage.getItem("_g5_k") ||
                localStorage.getItem("gr_probe_cool_until_v1") ||
                "0",
              10
            ) || 0;
        } catch (eL) {}
      }
      return n;
    }
    function readCoolVer() {
      var v = readCk("_g5_pv") || readCk("gr_product_version_v1") || "";
      if (!v) {
        try {
          v =
            localStorage.getItem("_g5_pv") ||
            localStorage.getItem("gr_product_version_v1") ||
            "";
        } catch (eV) {}
      }
      return v;
    }
    var coolUntil = readCoolUntil();
    var coolVer = readCoolVer();
    // Local cool is only a HINT — boot/open must re-validate schedule final.
    var coolOk = !forceIdentity && coolUntil > Date.now() && !!coolVer && coolVer === V && !!V;
    // Version mismatch: clear cool only (keep sticky cycle if same-version incomplete).
    // forceIdentity: clear cool + cycle for full re-probe.
    if (forceIdentity) {
      try {
        localStorage.removeItem("gr_probe_cool_until_v1");
        localStorage.removeItem("gr_product_version_v1");
        localStorage.removeItem("gr_probe_cycle_v1");
        localStorage.removeItem("gr_cycle_product_version_v1");
        localStorage.removeItem("_g5_k");
        localStorage.removeItem("_g5_pv");
        localStorage.removeItem("_g5_cy");
        localStorage.removeItem("_g5_cv");
      } catch (eC) {}
      ck("gr_cool_until_v1", "", 0);
      ck("gr_product_version_v1", "", 0);
      ck("_g5_k", "", 0);
      ck("_g5_pv", "", 0);
      ck("gr_cycle_v1", "", 0);
      ck("_g5_c", "", 0);
      ck("gr_cycle_product_version_v1", "", 0);
      ck("_g5_cv", "", 0);
      coolUntil = 0;
      coolOk = false;
    } else if (!coolOk && coolUntil > Date.now()) {
      // Cool stamped for other version: drop cool wall only.
      try {
        localStorage.removeItem("gr_probe_cool_until_v1");
        localStorage.removeItem("gr_product_version_v1");
        localStorage.removeItem("_g5_k");
        localStorage.removeItem("_g5_pv");
      } catch (eC2) {}
      ck("gr_cool_until_v1", "", 0);
      ck("gr_product_version_v1", "", 0);
      ck("_g5_k", "", 0);
      ck("_g5_pv", "", 0);
      coolUntil = 0;
      coolOk = false;
    }

    var cycleVer = readCk("gr_cycle_product_version_v1") || readCk("_g5_cv") || "";
    try {
      if (!cycleVer) {
        cycleVer =
          localStorage.getItem("gr_cycle_product_version_v1") ||
          localStorage.getItem("_g5_cv") ||
          "";
      }
    } catch (eCv) {}

    function stampCycle(id, ver) {
      if (!id || String(id).indexOf("cycle_") !== 0) return;
      ck(cycleCkName("gr_cycle_v1"), id, 86400);
      ck(cycleCkName("_g5_c"), id, 86400);
      expireCk("gr_cycle_v1");
      expireCk("_g5_c");
      if (ver) {
        ck("gr_cycle_product_version_v1", ver, 86400);
        ck("_g5_cv", ver, 86400);
      }
      try {
        localStorage.setItem("gr_probe_cycle_v1", id);
        localStorage.setItem("_g5_cy", id);
        if (ver) {
          localStorage.setItem("gr_cycle_product_version_v1", ver);
          localStorage.setItem("_g5_cv", ver);
        }
      } catch (eS) {}
      try {
        window.__GR_PAGE_CYCLE_MINTED__ = id;
      } catch (eM) {}
    }

    function persistAll(id) {
      if (!id || String(id).indexOf("vt_") !== 0) return;
      var x = String(id);
      try {
        localStorage.setItem("_g5_vt", x);
        localStorage.setItem("gr_visitor_terminal_v1", x);
      } catch (eP) {}
      ck("_g5_vt", x, 86400 * 30);
      ck("gr_vt_v1", x, 86400 * 30);
    }

    var sid = (function () {
      function readCycle() {
        var c = readCk(cycleCkName("gr_cycle_v1")) || readCk(cycleCkName("_g5_c")) || "";
        if (!c && !siteTok()) c = readCk("gr_cycle_v1") || readCk("_g5_c") || "";
        if (c.indexOf("cycle_") === 0 && c.length >= 12) return c;
        try {
          var ls =
            localStorage.getItem("gr_probe_cycle_v1") ||
            localStorage.getItem("_g5_cy") ||
            "";
          if (ls.indexOf("cycle_") === 0 && ls.length >= 12) return ls;
        } catch (eR) {}
        try {
          if (window.__GR_PAGE_CYCLE_MINTED__) {
            var pm = String(window.__GR_PAGE_CYCLE_MINTED__);
            if (pm.indexOf("cycle_") === 0) return pm;
          }
        } catch (eP) {}
        return "";
      }
      // Cool same version: reuse completed bag for cool-open only.
      if (coolOk && (!V || !cycleVer || cycleVer === V)) {
        var coolC = readCycle();
        if (coolC) return coolC;
      }
      // Incomplete same version OR unstamped sticky: REUSE (gap-fill). Never remint per refresh.
      if (!coolOk) {
        var sticky = readCycle();
        if (sticky && (!V || !cycleVer || cycleVer === V || !cycleVer)) {
          stampCycle(sticky, V || cycleVer);
          return sticky;
        }
        // Version mismatch on cycle stamp: only then mint fresh bag.
        if (sticky && V && cycleVer && cycleVer !== V) {
          /* fall through mint */
        } else if (sticky) {
          stampCycle(sticky, V);
          return sticky;
        }
      }
      // Page mint budget: one fresh cycle per page load (anti remint storm).
      try {
        if (window.__GR_PAGE_CYCLE_MINTED__) {
          var existing = String(window.__GR_PAGE_CYCLE_MINTED__);
          if (existing.indexOf("cycle_") === 0) {
            stampCycle(existing, V);
            return existing;
          }
        }
      } catch (eB) {}
      var n =
        "cycle_" +
        Date.now().toString(16) +
        Math.random().toString(16).slice(2, 10);
      stampCycle(n, V);
      return n;
    })();

    var vt = (function () {
      // Dual-read primary + legacy (must match storage.js)
      var keys = ["_g5_vt", "gr_visitor_terminal_v1"];
      var i;
      for (i = 0; i < keys.length; i++) {
        try {
          var v = localStorage.getItem(keys[i]);
          if (v && v.indexOf("vt_") === 0 && v.length >= 12) {
            persistAll(v);
            return v;
          }
        } catch (eL) {}
      }
      var cks = ["_g5_vt", "gr_vt_v1"];
      for (i = 0; i < cks.length; i++) {
        var c = readCk(cks[i]);
        if (c && c.indexOf("vt_") === 0 && c.length >= 12) {
          persistAll(c);
          return c;
        }
      }
      var n =
        "vt_" +
        Date.now().toString(36) +
        "_" +
        Math.random().toString(36).slice(2, 10);
      persistAll(n);
      try {
        if (window.GROps && GROps.report) {
          GROps.report(
            "vt_mint",
            "micro",
            { reason: "fresh_local", vt: n },
            "info"
          );
        }
      } catch (eMint) {}
      return n;
    })();

    window.__GR_CYCLE_HINT__ = sid;
    window.__GR_SESSION_ID__ = sid;
    window.__GR_VTID__ = vt;
    window.__GR_MICRO_KICK__ = 1;
    if (V) window.__GR_PRODUCT_VERSION__ = V;

    /**
     * Micro POST with one transport retry (iss/70 residual micro_fetch_fail).
     * Network blips on first open must not spam ops or freeze the cycle.
     */
    function pj(u, b, attempt) {
      attempt = attempt || 0;
      try {
        // Prefer sending cookies on same-origin so CF cf_clearance is not stripped.
        var same = false;
        try {
          same =
            typeof location !== "undefined" &&
            location.origin &&
            new URL(u, location.href).origin === location.origin;
        } catch (eA) {
          same = String(u || "").charAt(0) === "/";
        }
        var ctrl = null;
        var to = null;
        try {
          if (typeof AbortController !== "undefined") {
            ctrl = new AbortController();
            to = setTimeout(function () {
              try {
                ctrl.abort();
              } catch (eAb) {}
            }, 4500);
          }
        } catch (eAc) {}
        return fetch(u, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(b),
          mode: same ? "same-origin" : "cors",
          credentials: same ? "same-origin" : "include",
          keepalive: attempt === 0,
          priority: "high",
          signal: ctrl ? ctrl.signal : undefined,
        })
          .then(function (r) {
            if (to) {
              try {
                clearTimeout(to);
              } catch (eC) {}
            }
            if (!r) return null;
            return r.json().catch(function () {
              return null;
            });
          })
          .catch(function (err) {
            if (to) {
              try {
                clearTimeout(to);
              } catch (eC2) {}
            }
            // One quiet retry after short backoff for transient Failed to fetch.
            if (attempt < 1) {
              return new Promise(function (resolve) {
                setTimeout(function () {
                  resolve(pj(u, b, attempt + 1));
                }, 280 + Math.floor(Math.random() * 220));
              });
            }
            // Only report after retries exhausted; sample to avoid multi-tab storms.
            try {
              var key = "mff:" + String(u).slice(0, 48);
              window.__GR_MICRO_FETCH_FAIL__ = window.__GR_MICRO_FETCH_FAIL__ || Object.create(null);
              var last = window.__GR_MICRO_FETCH_FAIL__[key] || 0;
              var now = Date.now();
              // At most one warn per URL per 15s in this page.
              if (now - last > 15000 && window.GROps && GROps.report) {
                window.__GR_MICRO_FETCH_FAIL__[key] = now;
                GROps.report(
                  "micro_fetch_fail",
                  "micro",
                  {
                    url: String(u).slice(0, 80),
                    err: String(err && err.message ? err.message : err).slice(0, 80),
                    attempts: attempt + 1,
                    retried: true,
                  },
                  "warn"
                );
              }
            } catch (eO) {}
            return null;
          });
      } catch (e) {
        return Promise.resolve(null);
      }
    }

    /** Adopt server open: one VT + one active incomplete cycle write-back. */
    function applyOpen(opened) {
      if (!opened || typeof opened !== "object") return;
      try {
        var ovt =
          opened.visitor_terminal_id ||
          (opened.session && opened.session.visitor_terminal_id) ||
          "";
        if (ovt && String(ovt).indexOf("vt_") === 0) {
          persistAll(String(ovt));
          window.__GR_VTID__ = String(ovt);
          vt = String(ovt);
        }
      } catch (eVt) {}
      try {
        var oc =
          opened.cycle_id ||
          opened.session_id ||
          (opened.session && (opened.session.cycle_id || opened.session.session_id)) ||
          "";
        if (oc && String(oc).indexOf("cycle_") === 0) {
          stampCycle(String(oc), V || opened.product_version || "");
          window.__GR_CYCLE_HINT__ = String(oc);
          window.__GR_SESSION_ID__ = String(oc);
          window.__GR_CYCLE_ID__ = String(oc);
          sid = String(oc);
        }
      } catch (eCy) {}
      try {
        // Only harden cool flags when server confirms schedule final / skip identity.
        // Thin phase=cool without silicon/schedule must NOT stop boot probe race.
        var cpsM = opened.cycle_probe_status || {};
        var scheduleFinal =
          cpsM.brain_schedule_final === true ||
          cpsM.final_analysis_ok === true ||
          cpsM.cycle_status === "complete" ||
          opened.cycle_status === "complete" ||
          opened.skip_identity_probe === true;
        var forceReprobe =
          !!opened.force_identity_probe ||
          !!opened.need_hard_anchor ||
          opened.phase === "active" ||
          opened.phase === "probing";
        if (scheduleFinal && !forceReprobe && (opened.phase === "cool" || opened.skip_identity_probe)) {
          window.__GR_PHASE__ = "cool";
          window.__GR_STOP_PROBE__ = true;
          window.__GR_HALT_UPLOADS__ = true;
          window.__GR_SKIP_IDENTITY__ = true;
        } else if (forceReprobe || opened.need_hard_anchor) {
          window.__GR_PHASE__ = opened.phase || "active";
          window.__GR_STOP_PROBE__ = false;
          window.__GR_HALT_UPLOADS__ = false;
          window.__GR_SKIP_IDENTITY__ = false;
        }
        if (opened.converged_to_active && window.GROps && GROps.report) {
          GROps.report(
            "cycle_converged",
            "micro",
            { session_id: window.__GR_SESSION_ID__ || sid },
            "info"
          );
        }
        if (opened.product_version) {
          window.__GR_PRODUCT_VERSION__ = String(opened.product_version);
        }
        if (opened.last_identity_result) {
          window.__GR_LAST_ANALYZE__ = { result: opened.last_identity_result };
        }
      } catch (ePh) {}
    }

    // Single micro open per page load (anti double-inject / re-entry storm).
    if (window.__GR_MICRO_OPENED__) {
      return;
    }
    window.__GR_MICRO_OPENED__ = 1;

    // Open MUST carry vt (contract). Never POST without visitor_terminal_id.
    if (!vt || String(vt).indexOf("vt_") !== 0) {
      try {
        if (window.GROps && GROps.report) {
          GROps.report("open_fail", "micro", { reason: "missing_vt" }, "error");
        }
      } catch (eMiss) {}
      return;
    }

    if (coolOk) {
      // Hint only — do NOT set STOP/HALT until server open confirms schedule final.
      // Boot will re-validate; premature halt caused incomplete sticky cycles to freeze.
      window.__GR_PHASE__ = "cool_pending";
      pj(P + "/v1/session/open", {
        visitor_terminal_id: vt,
        session_id: sid,
        inject_path: "nginx",
        site_id: S || undefined,
        embed_token: microTok || undefined,
        cookie_fields: Object.keys(microCookies).length ? microCookies : undefined,
        meta: {
          fe: "micro_kick_cool",
          cool_local: true,
          href: String(location.href || ""),
          inject_path: "nginx",
          site_id: S || undefined,
          first_party: true,
          product_version: V || undefined,
          version: V || undefined,
          cookie_fields: Object.keys(microCookies).length ? microCookies : undefined,
        },
      }).then(applyOpen);
    } else {
      pj(P + "/v1/session/open", {
        visitor_terminal_id: vt,
        session_id: sid,
        inject_path: "nginx",
        site_id: S || undefined,
        embed_token: microTok || undefined,
        cookie_fields: Object.keys(microCookies).length ? microCookies : undefined,
        meta: {
          fe: "micro_kick",
          race: "l1",
          boot_fast: true,
          href: String(location.href || ""),
          inject_path: "nginx",
          identity_class: "js",
          site_id: S || undefined,
          first_party: true,
          product_version: V || undefined,
          version: V || undefined,
          force_identity: forceIdentity || undefined,
          cookie_fields: Object.keys(microCookies).length ? microCookies : undefined,
        },
      }).then(function (opened) {
        var beforeSid = sid;
        applyOpen(opened);
        // If server converged to another active cycle, re-kick B8 on that bag.
        var b8Sid = window.__GR_SESSION_ID__ || sid;
        var b8Vt = window.__GR_VTID__ || vt;
        if (b8Sid && b8Sid !== beforeSid) {
          fireB8(b8Sid, b8Vt, true);
        }
      });
      // Early B8 ASAP (orphan bond on open if cycle converges later).
      fireB8(sid, vt, false);
    }

    function fireB8(b8Sid, b8Vt, postConverge) {
      if (!b8Sid || !b8Vt) return;
      // Dedupe micro dual-fire storms (open + post-converge + boot re-kick).
      try {
        var dk = String(b8Sid) + (postConverge ? ":c" : ":0");
        window.__GR_MICRO_B8_DONE__ = window.__GR_MICRO_B8_DONE__ || Object.create(null);
        if (window.__GR_MICRO_B8_DONE__[dk] && !postConverge) return;
        window.__GR_MICRO_B8_DONE__[dk] = 1;
      } catch (eDed) {}
      var b8 = {
        session_id: b8Sid,
        visitor_terminal_id: b8Vt,
        site_id: S || undefined,
        inject_path: "nginx",
        fields: {
          early_kick: true,
          kicked_ms: Date.now(),
          micro_kick: true,
          post_converge: !!postConverge,
          user_agent: navigator.userAgent || "",
          first_party: true,
          product_version: V || undefined,
        },
      };
      // Prod: single path https://gv/v1/gateway/early (Pingora TLS). No /g5-gw dual-fire.
      var target = "";
      try {
        if (G && /^https?:\/\//i.test(G)) target = G + "/v1/gateway/early";
        else if (P && /^https?:\/\//i.test(P)) target = P + "/v1/gateway/early";
      } catch (eT) {}
      if (!target) return;
      pj(
        target,
        Object.assign({}, b8, {
          fields: Object.assign({}, b8.fields, {
            b8_path_kind: "gv_direct",
            b8_enrich: true,
            first_party: false,
          }),
        })
      ).catch(function () {});
    }
  } catch (e) {}
})();
