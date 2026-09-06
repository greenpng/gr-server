/**
 * GRSeal — session seal v2 (strict) + legacy v1 lab path.
 *
 * v2:
 *  - epoch/suite-bound grant key (server-derived)
 *  - envelope metadata: fe_epoch, suite_id, wasm_module_id, challenge_bind, pack_set_hash
 *  - AEAD via WebCrypto AES-256-GCM; KDF labels v2
 *  - bind digests MUST come from WASM module (wm-v2-s2-*) unless lab break-glass
 *
 * Master SEAL_SECRET never embedded.
 */
(function (global) {
  "use strict";

  var grant = null;
  var requireSealed = false;
  var wasmApi = null; // { module_id, suite_id, challenge_bind, pack_set_hash, attest }
  var wasmLoadP = null;
  var ALGO_DEFLATE = "aes256gcm+hmacsha256+deflate";
  var ALGO_IDENTITY = "aes256gcm+hmacsha256+identity";
  var SUITE_S2 = "s2-aesgcm-hkdf-v1";
  var WASM_ID = "wm-v2-s2-20260806";
  // Patched by scripts/build_fe_all.sh from fe/gr_seal_v2.wasm (0 = unset).
  var WASM_EXPECT_LEN = 28961;
  var WASM_EXPECT_SHA256 = "c5b97b9ad21c3dc3a47b02d55f1409dc1d4442409b14b2a4dd81cd0d45ef4409";

  function b64ToBytes(b64) {
    var bin = atob(b64);
    var out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function bytesToB64(u8) {
    var s = "";
    var chunk = 0x8000;
    for (var i = 0; i < u8.length; i += chunk) {
      s += String.fromCharCode.apply(null, u8.subarray(i, i + chunk));
    }
    return btoa(s);
  }

  function concatBytes(a, b) {
    var o = new Uint8Array(a.length + b.length);
    o.set(a, 0);
    o.set(b, a.length);
    return o;
  }

  function utf8(s) {
    return new TextEncoder().encode(s);
  }

  async function sha256(data) {
    var buf = await crypto.subtle.digest("SHA-256", data);
    return new Uint8Array(buf);
  }

  function toHex(u8) {
    var HEX = "0123456789abcdef";
    var s = "";
    for (var i = 0; i < u8.length; i++) {
      s += HEX[u8[i] >> 4] + HEX[u8[i] & 0xf];
    }
    return s;
  }

  /** v1 labels (lab only) */
  async function deriveKeysV1(secretBytes) {
    var enc = await sha256(concatBytes(utf8("gr-seal-enc-v1"), secretBytes));
    var mac = await sha256(concatBytes(utf8("gr-seal-mac-v1"), secretBytes));
    return { enc: enc, mac: mac };
  }

  /** v2 labels */
  async function deriveKeysV2(secretBytes) {
    var enc = await sha256(concatBytes(utf8("gr-seal-enc-v2"), secretBytes));
    var mac = await sha256(concatBytes(utf8("gr-seal-mac-v2"), secretBytes));
    return { enc: enc, mac: mac };
  }

  async function hmacSha256B64(macKeyBytes, parts) {
    var key = await crypto.subtle.importKey(
      "raw",
      macKeyBytes,
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign"]
    );
    var totalLen = 0;
    for (var i = 0; i < parts.length; i++) totalLen += parts[i].length;
    var msg = new Uint8Array(totalLen);
    var off = 0;
    for (var j = 0; j < parts.length; j++) {
      msg.set(parts[j], off);
      off += parts[j].length;
    }
    var sig = await crypto.subtle.sign("HMAC", key, msg);
    return bytesToB64(new Uint8Array(sig));
  }

  async function preparePlainForAes(u8) {
    if (typeof CompressionStream !== "undefined") {
      try {
        var compressed;
        if (typeof Blob !== "undefined" && typeof Response !== "undefined") {
          var stream = new Blob([u8]).stream().pipeThrough(new CompressionStream("deflate-raw"));
          compressed = new Uint8Array(await new Response(stream).arrayBuffer());
        } else {
          var cs = new CompressionStream("deflate-raw");
          var reader = cs.readable.getReader();
          var writer = cs.writable.getWriter();
          var readP = (async function () {
            var chunks = [];
            var total = 0;
            for (;;) {
              var n = await reader.read();
              if (n.done) break;
              chunks.push(n.value);
              total += n.value.length;
            }
            var out = new Uint8Array(total);
            var o = 0;
            for (var i = 0; i < chunks.length; i++) {
              out.set(chunks[i], o);
              o += chunks[i].length;
            }
            return out;
          })();
          await writer.write(u8);
          await writer.close();
          compressed = await readP;
        }
        if (compressed && compressed.length) {
          return { bytes: compressed, alg: ALGO_DEFLATE, deflated: true };
        }
      } catch (eDef) {}
    }
    return { bytes: u8, alg: ALGO_IDENTITY, deflated: false };
  }

  function grantIsV2(g) {
    if (!g) return false;
    return Number(g.seal_protocol || 0) >= 2 || g.algo === "gr_session_seal_v2" || !!g.suite_id;
  }

  function setGrant(g) {
    if (!g || typeof g !== "object") return;
    grant = {
      key_b64: g.key_b64 || g.keyB64 || "",
      exp_ms: Number(g.exp_ms || g.expMs || 0),
      session_id: g.session_id || g.sessionId || "",
      key_mode: g.key_mode || "session",
      seal_protocol: Number(g.seal_protocol || (g.algo === "gr_session_seal_v2" ? 2 : 1)),
      fe_epoch: g.fe_epoch || g.product_version || "",
      suite_id: g.suite_id || SUITE_S2,
      wasm_module_id: g.wasm_module_id || WASM_ID,
      wasm_url: g.wasm_url || g.wasm_url_flat || "",
      require_wasm: g.require_wasm !== false,
      challenge_seed: g.challenge_seed || "",
      algo: g.algo || "",
    };
    try {
      global.__GR_SEAL_GRANT__ = grant;
      global.__GR_BOOT__ = global.__GR_BOOT__ || {};
      global.__GR_BOOT__.seal_grant = grant;
      if (grant.fe_epoch) {
        global.__GR_FE_EPOCH__ = grant.fe_epoch;
      }
    } catch (e) {}
    // Kick WASM load as soon as grant arrives
    if (grantIsV2(grant)) {
      ensureWasm(grant).catch(function () {});
    }
  }

  function setRequireSealed(on) {
    requireSealed = !!on;
    try {
      global.__GR_REQUIRE_SEALED__ = requireSealed;
      global.__GR_BOOT__ = global.__GR_BOOT__ || {};
      if (requireSealed) global.__GR_BOOT__.require_sealed_ingest = true;
    } catch (e) {}
  }

  function adoptFromGlobals() {
    try {
      if (global.__GR_REQUIRE_SEALED__) requireSealed = true;
      var b = global.__GR_BOOT__ || {};
      if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) {
        requireSealed = true;
      }
      var g = global.__GR_SEAL_GRANT__ || b.seal_grant || null;
      if (g && (g.key_b64 || g.keyB64)) {
        if (!grant || !grant.key_b64) {
          setGrant(g);
        } else {
          var gExp = Number(g.exp_ms || g.expMs || 0);
          var gSid = g.session_id || g.sessionId || "";
          if (gSid && grant.session_id && gSid !== grant.session_id) {
            setGrant(g);
          } else if (gExp && (!grant.exp_ms || gExp > grant.exp_ms)) {
            setGrant(g);
          } else if (!grant.key_b64 && (g.key_b64 || g.keyB64)) {
            setGrant(g);
          }
        }
      }
      // bootstrap seal_v2 meta
      var meta = b.seal_v2 || (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) || null;
      if (meta && meta.wasm_url && grant && !grant.wasm_url) {
        grant.wasm_url = meta.wasm_url;
      }
    } catch (eAd) {}
  }

  function isRequireSealed() {
    adoptFromGlobals();
    if (requireSealed) return true;
    try {
      if (global.__GR_FORCE_PLAIN_INGEST__) return false;
      if (global.__GR_REQUIRE_SEALED__) return true;
      var b = global.__GR_BOOT__ || {};
      if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) return true;
      if (global.__GR_SEEN_SEALED_REQUIRED__) return true;
      if (global.__GR_FIRST_PARTY__ || b.first_party || b.firstParty) {
        if (b.env_id && String(b.env_id).indexOf("prod") === 0) return true;
        if (b.injectPath === "nginx" || b.inject_path === "nginx") return true;
      }
    } catch (e) {}
    return false;
  }

  function grantRawValid() {
    adoptFromGlobals();
    if (!grant || !grant.key_b64) return false;
    if (grant.exp_ms && Date.now() > grant.exp_ms) return false;
    return true;
  }

  function grantValid(sessionId) {
    if (!grantRawValid()) return false;
    if (sessionId && grant.session_id && grant.session_id !== sessionId) return false;
    return true;
  }

  function resolveSealSessionId(bodySid) {
    adoptFromGlobals();
    var want = String(bodySid || "");
    if (grantValid(want)) return want;
    var candidates = [];
    try {
      if (global.__GR_SESSION_ID__) candidates.push(String(global.__GR_SESSION_ID__));
      if (global.__GR_CYCLE_ID__) candidates.push(String(global.__GR_CYCLE_ID__));
      if (grant && grant.session_id) candidates.push(String(grant.session_id));
    } catch (eC) {}
    for (var i = 0; i < candidates.length; i++) {
      if (candidates[i] && grantValid(candidates[i])) return candidates[i];
    }
    if (grantRawValid() && grant.session_id) return String(grant.session_id);
    return want;
  }

  function waitForGrant(sessionId, timeoutMs) {
    var need = false;
    try {
      need = isRequireSealed();
    } catch (eN) {}
    timeoutMs = timeoutMs == null ? (need ? 16000 : 4000) : timeoutMs;
    adoptFromGlobals();
    if (grantValid(sessionId) || (sessionId && resolveSealSessionId(sessionId) && grantValid(resolveSealSessionId(sessionId)))) {
      return Promise.resolve(true);
    }
    if (grantRawValid()) return Promise.resolve(true);
    if (!need && !sessionId) return Promise.resolve(false);
    var start = Date.now();
    return new Promise(function (resolve) {
      function tick() {
        adoptFromGlobals();
        var sid = resolveSealSessionId(sessionId);
        if (grantValid(sid) || grantRawValid()) return resolve(true);
        if (Date.now() - start > timeoutMs) return resolve(false);
        setTimeout(tick, 35);
      }
      tick();
    });
  }

  /**
   * Resolve seal static URL for fe_load mode:
   * - first_party → same-origin /g5/dist/... (www)
   * - pv → https://pv.../dist/... (never /g5 on pv; never gv host)
   */
  function resolveSealAssetUrl(pathOrUrl) {
    var u = String(pathOrUrl || "");
    var b0 = global.__GR_BOOT__ || {};
    var assetRoot = String(
      b0.assetBase ||
        b0.script_base ||
        b0.first_party_path ||
        global.__GR_ASSET_BASE__ ||
        "/g5"
    ).replace(/\/$/, "");
    if (assetRoot.indexOf("/dist/v/") > 0 || /\/dist$/.test(assetRoot)) {
      assetRoot = assetRoot.replace(/\/dist\/.*$/, "").replace(/\/dist$/, "") || "/g5";
    }
    var fe = String(b0.fe_load || b0.feLoad || (global.__GR_FIRST_PARTY__ ? "first_party" : "") || "").toLowerCase();
    var pvMode = fe === "pv" || /^https?:\/\/pv\./i.test(assetRoot);
    // Never treat apiBase/gv as asset root
    try {
      var api = String(b0.apiBase || "").replace(/\/$/, "");
      if (api && assetRoot === api && /^https?:\/\/gv\./i.test(api)) {
        assetRoot = pvMode ? assetRoot : "/g5";
      }
    } catch (eApi) {}

    function underRoot(path) {
      var p = String(path || "");
      if (!p) return "";
      if (pvMode || /^https?:\/\//i.test(assetRoot)) {
        if (p.indexOf("/g5/") === 0 || p === "/g5") p = p.replace(/^\/g5(?=\/|$)/, "") || "/";
        try {
          return new URL(p, assetRoot + "/").href;
        } catch (eN) {
          return assetRoot.replace(/\/$/, "") + (p.charAt(0) === "/" ? p : "/" + p);
        }
      }
      // first_party: force document-origin absolute so dynamic import never follows gv
      if (p.charAt(0) === "/") {
        try {
          if (typeof location !== "undefined" && location.origin) {
            return location.origin + p;
          }
        } catch (eL) {}
        return p;
      }
      return (assetRoot || "/g5") + "/" + p.replace(/^\//, "");
    }

    if (!u) return underRoot("/g5/dist/gr_seal_v2.wasm");
    if (/^https?:\/\//i.test(u)) {
      try {
        var abs = new URL(u);
        if (/^gv\./i.test(abs.hostname)) {
          // remap onto asset root
          var pathG = abs.pathname || "/";
          if (pathG.indexOf("/g5/") === 0) pathG = pathG.replace(/^\/g5(?=\/|$)/, "") || "/";
          return underRoot(pathG);
        }
        return abs.href;
      } catch (eA) {
        return u;
      }
    }
    return underRoot(u.charAt(0) === "/" ? u : "/" + u);
  }

  function wasmEnvSupported() {
    try {
      return (
        typeof WebAssembly !== "undefined" &&
        typeof WebAssembly.instantiate === "function" &&
        typeof WebAssembly.Module === "function"
      );
    } catch (eW) {
      return false;
    }
  }

  function looksLikeWasmBinary(u8) {
    // \0asm magic
    return (
      u8 &&
      u8.length >= 8 &&
      u8[0] === 0x00 &&
      u8[1] === 0x61 &&
      u8[2] === 0x73 &&
      u8[3] === 0x6d
    );
  }

  function wasmExpectFromBoot(g) {
    var meta =
      (g && g.seal_v2) ||
      (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2) ||
      (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) ||
      {};
    var len =
      Number(meta.wasm_bytes || meta.wasm_len || g.wasm_bytes || WASM_EXPECT_LEN || 0) || 0;
    var sha = String(
      meta.wasm_sha256 || meta.wasm_sha || g.wasm_sha256 || WASM_EXPECT_SHA256 || ""
    )
      .trim()
      .toLowerCase();
    return { len: len, sha256: sha };
  }

  async function sha256Hex(buf) {
    var dig = await crypto.subtle.digest("SHA-256", buf);
    var u8 = new Uint8Array(dig);
    var hex = "";
    for (var i = 0; i < u8.length; i++) {
      hex += (u8[i] + 256).toString(16).slice(1);
    }
    return hex;
  }

  /**
   * Fetch wasm bytes with integrity checks. Always arrayBuffer-first
   * (small seal module; MIME/truncation/HTML-intercept is common on bots/CF).
   */
  async function fetchWasmBytes(wasmUrl, g) {
    var resp = await fetch(wasmUrl, {
      cache: "no-cache",
      credentials: "same-origin",
      mode: "cors",
    });
    if (!resp.ok) throw new Error("wasm_http_" + resp.status);
    var ct = "";
    try {
      ct = String(resp.headers.get("Content-Type") || "").toLowerCase();
    } catch (eC) {}
    var buf = await resp.arrayBuffer();
    var u8 = new Uint8Array(buf);
    if (!looksLikeWasmBinary(u8)) {
      // HTML/JSON intercept (login wall / CF / 404 page) → not a wasm module
      var snip = "";
      try {
        snip = new TextDecoder().decode(u8.subarray(0, 48)).replace(/\s+/g, " ");
      } catch (eD) {}
      throw new Error(
        "wasm_not_binary:ct=" +
          (ct || "?").slice(0, 40) +
          ";len=" +
          u8.length +
          ";snip=" +
          snip.slice(0, 40)
      );
    }
    var expect = wasmExpectFromBoot(g || {});
    if (expect.len > 0 && u8.length !== expect.len) {
      throw new Error(
        "wasm_len_mismatch:got=" + u8.length + ";want=" + expect.len
      );
    }
    if (expect.sha256 && expect.sha256.length >= 32) {
      var got = await sha256Hex(buf);
      if (got !== expect.sha256) {
        throw new Error(
          "wasm_sha256_mismatch:got=" + got.slice(0, 16) + ";want=" + expect.sha256.slice(0, 16)
        );
      }
    }
    return buf;
  }

  /**
   * Load WASM seal helper (required for v2 under require_wasm).
   */
  function ensureWasm(g) {
    if (wasmApi) return Promise.resolve(wasmApi);
    if (wasmLoadP) return wasmLoadP;
    // Permanent fail short-circuit (crawlers without WASM / blocked wasm assets).
    if (global.__GR_SEAL_WASM_DEAD__) {
      return Promise.reject(
        new Error(String(global.__GR_SEAL_WASM_DEAD_ERR__ || "seal_wasm_required: dead"))
      );
    }
    g = g || grant || {};
    var url =
      g.wasm_url ||
      (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2 && global.__GR_BOOT__.seal_v2.wasm_url) ||
      (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2 && global.__GR_MANIFEST__.seal_v2.wasm_url) ||
      "";
    if (!url) {
      // Standard C: never use /dist/v/<version>/ — prefer bootstrap hashed URLs.
      try {
        var meta0 =
          (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.seal_v2) ||
          (global.__GR_BOOT__ && global.__GR_BOOT__.seal_v2) ||
          null;
        if (meta0 && meta0.wasm_url) url = String(meta0.wasm_url);
      } catch (eM0) {}
      if (!url) {
        var gen0 = "";
        try {
          gen0 = String(
            (global.__GR_MANIFEST__ && global.__GR_MANIFEST__.asset_gen) ||
              global.__GR_ASSET_GEN__ ||
              ""
          );
        } catch (eG0) {}
        url = gen0
          ? "/g5/dist/gr_seal_v2." + gen0 + ".wasm"
          : "/g5/dist/gr_seal_v2.wasm";
      }
    }
    // Collapse sticky version paths from old grants.
    url = String(url).replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    url = resolveSealAssetUrl(url);
    // Prefer ES module glue when present (preserve content-hash token if any).
    var glue = "";
    try {
      var metaL =
        (g && g.loader_url) ||
        (global.__GR_MANIFEST__ &&
          global.__GR_MANIFEST__.seal_v2 &&
          global.__GR_MANIFEST__.seal_v2.loader_url) ||
        (global.__GR_BOOT__ &&
          global.__GR_BOOT__.seal_v2 &&
          global.__GR_BOOT__.seal_v2.loader_url) ||
        "";
      if (metaL) glue = String(metaL);
    } catch (eML) {}
    if (!glue) {
      // Map hashed wasm → hashed loader when possible: gr_seal_v2.<h>.wasm → loader.<h>.js
      var wm = String(url).match(/gr_seal_v2(?:\.([a-f0-9]{8,16}))?\.wasm/i);
      if (wm && wm[1]) {
        glue = String(url).replace(
          /gr_seal_v2(?:\.[a-f0-9]{8,16})?\.wasm.*/i,
          "gr_seal_v2_loader." + wm[1] + ".js"
        );
      } else {
        glue =
          (url.indexOf(".wasm") > 0
            ? url.replace(/gr_seal_v2(?:\.[a-f0-9]{8,16})?\.wasm.*/i, "gr_seal_v2_loader.js")
            : "") || url.replace(/\.wasm.*/, "_loader.js");
      }
    }
    glue = String(glue).replace(/\/dist\/v\/[^/]+\/(?:g\/[^/]+\/)?/, "/dist/");
    glue = resolveSealAssetUrl(glue);

    wasmLoadP = (async function () {
      if (!wasmEnvSupported()) {
        var eUn = new Error("seal_wasm_unsupported: WebAssembly API missing");
        global.__GR_SEAL_WASM_DEAD__ = 1;
        global.__GR_SEAL_WASM_DEAD_ERR__ = eUn.message;
        global.__GR_SEAL_WASM_CAP__ = { ok: false, reason: "no_webassembly_api" };
        throw eUn;
      }
      global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "api_present" };
      var lastErr = null;
      // Path A: wasm-bindgen ES module glue (preferred — full exports)
      try {
        var loaderUrl = glue;
        if (!loaderUrl) {
          loaderUrl = resolveSealAssetUrl("/g5/dist/gr_seal_v2_loader.js");
        }
        var mod = await import(/* webpackIgnore: true */ loaderUrl);
        // Bytes-first: validate magic/len/sha256, then initSync / default(bytes).
        // Avoid instantiateStreaming (MIME/truncation/CF HTML) for ~29KB seal.
        var bytesValidated = await fetchWasmBytes(url, g);
        if (mod && typeof mod.initSync === "function") {
          mod.initSync({ module: bytesValidated });
        } else if (mod && mod.default) {
          await mod.default(bytesValidated);
        } else {
          throw new Error("wasm_glue_no_init");
        }
        var api = {
          module_id: mod.module_id ? mod.module_id() : WASM_ID,
          suite_id: mod.suite_id ? mod.suite_id() : SUITE_S2,
          challenge_bind: function (sid, seed, epoch) {
            return mod.challenge_bind(sid, seed, epoch);
          },
          pack_set_hash: function (joined) {
            return mod.pack_set_hash(joined);
          },
        };
        wasmApi = api;
        global.__GR_SEAL_WASM__ = api;
        global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "glue_ok", url: String(url).slice(0, 120) };
        return api;
      } catch (eImp) {
        lastErr = eImp;
        // Path B: raw instantiate from validated bytes (no glue exports → still fail closed)
        try {
          var bytes = await fetchWasmBytes(url, g);
          var result = await WebAssembly.instantiate(bytes, {});
          var exp = result.instance && result.instance.exports;
          if (!exp || typeof exp.challenge_bind !== "function") {
            throw new Error("wasm_no_exports");
          }
          var apiRaw = {
            module_id: WASM_ID,
            suite_id: SUITE_S2,
            challenge_bind: function (sid, seed, epoch) {
              return exp.challenge_bind(sid, seed, epoch);
            },
            pack_set_hash: function (joined) {
              return exp.pack_set_hash(joined);
            },
          };
          wasmApi = apiRaw;
          global.__GR_SEAL_WASM__ = apiRaw;
          global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "raw_instantiate", url: String(url).slice(0, 120) };
          return apiRaw;
        } catch (eRaw) {
          lastErr = eRaw || eImp;
          // Path C: lab-only pure JS bind (identical digests) — only if allowed
          var allowJs =
            global.__GR_SEAL_ALLOW_JS_BIND__ ||
            (global.__GR_BOOT__ && global.__GR_BOOT__.seal_allow_js_bind);
          if (!allowJs && isRequireSealed()) {
            var msg =
              "seal_wasm_required: " +
              String((lastErr && lastErr.message) || lastErr || eImp || "load_failed");
            global.__GR_SEAL_WASM_DEAD__ = 1;
            global.__GR_SEAL_WASM_DEAD_ERR__ = msg.slice(0, 200);
            global.__GR_SEAL_WASM_CAP__ = {
              ok: false,
              reason: "load_failed",
              err: msg.slice(0, 160),
              url: String(url).slice(0, 120),
            };
            // Clear load promise so... actually keep dead short-circuit via DEAD flag
            throw new Error(msg);
          }
          wasmApi = pureJsBindApi();
          global.__GR_SEAL_WASM__ = wasmApi;
          global.__GR_SEAL_WASM_JS_FALLBACK__ = 1;
          global.__GR_SEAL_WASM_CAP__ = { ok: true, reason: "js_fallback" };
          return wasmApi;
        }
      }
    })().catch(function (eFail) {
      // Allow one future retry only if not marked permanent dead.
      if (!global.__GR_SEAL_WASM_DEAD__) wasmLoadP = null;
      throw eFail;
    });

    return wasmLoadP;
  }

  /** Pure JS bind matching Rust/WASM digest formulas (lab fallback only). */
  function pureJsBindApi() {
    async function shaHex(parts) {
      var total = 0;
      for (var i = 0; i < parts.length; i++) total += parts[i].length;
      var msg = new Uint8Array(total);
      var o = 0;
      for (var j = 0; j < parts.length; j++) {
        msg.set(parts[j], o);
        o += parts[j].length;
      }
      return toHex(await sha256(msg));
    }
    return {
      module_id: WASM_ID,
      suite_id: SUITE_S2,
      challenge_bind: function (sid, seed, epoch) {
        // sync wrapper using cached promise — make async at call site
        return null;
      },
      challenge_bind_async: async function (sid, seed, epoch) {
        return shaHex([
          utf8("gr-challenge-bind-v2|"),
          utf8(String(sid || "")),
          utf8("|"),
          utf8(String(seed || "")),
          utf8("|"),
          utf8(String(epoch || "")),
        ]);
      },
      pack_set_hash: function () {
        return null;
      },
      pack_set_hash_async: async function (joined) {
        return shaHex([utf8("gr-pack-set-v2|"), utf8(String(joined || ""))]);
      },
      async_mode: true,
    };
  }

  async function resolveBind(api, kind, a, b, c) {
    if (!api) throw new Error("seal_wasm_missing");
    if (kind === "challenge") {
      if (api.async_mode && api.challenge_bind_async) return api.challenge_bind_async(a, b, c);
      if (typeof api.challenge_bind === "function") {
        var r = api.challenge_bind(a, b, c);
        if (r && typeof r.then === "function") return r;
        if (r) return r;
      }
      // JS fallback digest
      return (await pureJsBindApi().challenge_bind_async(a, b, c));
    }
    if (kind === "pack") {
      if (api.async_mode && api.pack_set_hash_async) return api.pack_set_hash_async(a);
      if (typeof api.pack_set_hash === "function") {
        var r2 = api.pack_set_hash(a);
        if (r2 && typeof r2.then === "function") return r2;
        if (r2) return r2;
      }
      return pureJsBindApi().pack_set_hash_async(a);
    }
    throw new Error("bind_kind");
  }

  function buildPackSetParts(body) {
    var epoch =
      (grant && grant.fe_epoch) ||
      global.__GR_SERVER_PRODUCT_VERSION__ ||
      global.__GR_PRODUCT_VERSION__ ||
      "";
    var bid = String((body && body.batch_id) || "");
    var fields =
      (body && body.payload && body.payload.fields) ||
      (body && body.fields) ||
      {};
    var algo = String(fields.cpu_loop_algo || fields.cpu_loop_algo_build || "");
    var feImpl = String(fields.fe_impl_version || fields.fe_packs_version || "");
    var hard = "";
    try {
      hard = String(global.__GR_FE_HARD_IMPL__ || (global.__GR_FE_IMPL__ && global.__GR_FE_IMPL__.hard) || "");
    } catch (eH) {}
    // probe_dag_v2 decision (design §9.3): version + engine claim enter the
    // pack-set binding so the server can explain "why was field X not executed"
    // from the sealed envelope alone, without trusting any later batch.
    var dagVer = "";
    var dagEngine = "";
    try {
      var consumed = global.__GR_DAG_V2_CONSUMED__ || null;
      if (consumed) {
        dagVer = String(consumed.version || 0);
        dagEngine = String(consumed.engine || "unknown");
      }
    } catch (eDag) {}
    // Join with | matching server compute_pack_set_hash parts
    return [epoch, bid, algo || "-", feImpl || hard || "-", WASM_ID, dagVer || "0", dagEngine || "unknown"].join("|");
  }

  function challengeSeedMaterial() {
    try {
      if (grant && grant.challenge_seed) return String(grant.challenge_seed);
      var b = global.__GR_BOOT__ || {};
      if (b.challenge_seed) return String(b.challenge_seed);
      if (global.__GR_CHALLENGE_SEED__) return String(global.__GR_CHALLENGE_SEED__);
      // open may store on cycle
      if (global.__GR_OPEN__ && global.__GR_OPEN__.challenge_seed) {
        return String(global.__GR_OPEN__.challenge_seed);
      }
    } catch (e) {}
    // Deterministic non-empty bind even without seed (still epoch+session bound in WASM formula)
    return "open_seed_pending";
  }

  async function sealIngestBodyV2(body) {
    if (!global.crypto || !crypto.subtle) throw new Error("webcrypto_unavailable");
    adoptFromGlobals();
    var sid = resolveSealSessionId(String((body && body.session_id) || ""));
    if (body && sid && body.session_id !== sid) body.session_id = sid;
    if (!grantValid(sid)) throw new Error("seal_grant_missing");
    if (!grantIsV2(grant)) throw new Error("seal_grant_not_v2");

    var api = await ensureWasm(grant);
    var epoch = grant.fe_epoch || global.__GR_PRODUCT_VERSION__ || "";
    var suite = grant.suite_id || SUITE_S2;
    var wasmId = (api && api.module_id) || grant.wasm_module_id || WASM_ID;
    var seed = challengeSeedMaterial();
    var chBind = await resolveBind(api, "challenge", sid, seed, epoch);
    var packParts = buildPackSetParts(body);
    var psh = await resolveBind(api, "pack", packParts);

    // Stamp client honesty fields on payload when missing
    try {
      body.payload = body.payload || {};
      body.payload.fields = body.payload.fields || {};
      var f = body.payload.fields;
      if (!f.fe_impl_version) {
        f.fe_impl_version =
          global.__GR_FE_IMPL_VERSION__ ||
          (global.__GR_FE_IMPL__ && global.__GR_FE_IMPL__.content) ||
          epoch;
      }
      if (!f.fe_packs_version) f.fe_packs_version = f.fe_impl_version;
      f.client_fe_epoch = epoch;
      f.seal_suite_id = suite;
      f.seal_wasm_module_id = wasmId;
      // DAG v2 decision stamp (best-effort; missing when no plan was applied yet).
      var dagStamp = (global.__GR_DAG_V2_CONSUMED__) || null;
      if (dagStamp) {
        f.dag_v2_version = dagStamp.version || 0;
        f.dag_v2_engine = dagStamp.engine || "unknown";
      }
    } catch (eSt) {}

    var secret = b64ToBytes(grant.key_b64);
    var keys = await deriveKeysV2(secret);
    var plain = utf8(JSON.stringify(body));
    var pre = await preparePlainForAes(plain);
    var nonce = new Uint8Array(12);
    crypto.getRandomValues(nonce);
    var aesKey = await crypto.subtle.importKey("raw", keys.enc, { name: "AES-GCM" }, false, [
      "encrypt",
    ]);
    var ctBuf = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      aesKey,
      pre.bytes
    );
    var ct = new Uint8Array(ctBuf);
    var vt =
      (body.payload && body.payload.fields && body.payload.fields.visitor_terminal_id) ||
      (body.payload && body.payload.visitor_terminal_id) ||
      global.__GR_VTID__ ||
      "";
    try {
      if (!vt && global.GRStorage && GRStorage.visitorTerminalId) {
        vt = GRStorage.visitorTerminalId() || "";
      }
    } catch (e) {}
    var env = {
      v: 2,
      alg: pre.alg,
      session_id: sid,
      visitor_terminal_id: String(vt || "vt_unknown"),
      batch_id: String((body && body.batch_id) || ""),
      nonce_b64: bytesToB64(nonce),
      ciphertext_b64: bytesToB64(ct),
      sig_b64: "",
      key_mode: "session",
      seal_exp_ms: grant.exp_ms || 0,
      suite_id: suite,
      fe_epoch: epoch,
      wasm_module_id: wasmId,
      challenge_bind: String(chBind || ""),
      pack_set_hash: String(psh || ""),
    };
    env.sig_b64 = await hmacSha256B64(keys.mac, [
      utf8("v2|"),
      utf8(env.session_id),
      utf8("|"),
      utf8(env.visitor_terminal_id),
      utf8("|"),
      utf8(env.batch_id),
      utf8("|"),
      utf8(env.suite_id),
      utf8("|"),
      utf8(env.fe_epoch),
      utf8("|"),
      utf8(env.wasm_module_id),
      utf8("|"),
      utf8(env.challenge_bind),
      utf8("|"),
      utf8(env.pack_set_hash),
      utf8("|"),
      utf8(env.nonce_b64),
      utf8("|"),
      utf8(env.ciphertext_b64),
    ]);
    return env;
  }

  /** Legacy v1 seal (lab only when grant is v1). */
  async function sealIngestBodyV1(body) {
    if (!global.crypto || !crypto.subtle) throw new Error("webcrypto_unavailable");
    adoptFromGlobals();
    var sid = resolveSealSessionId(String((body && body.session_id) || ""));
    if (body && sid && body.session_id !== sid) body.session_id = sid;
    if (!grantValid(sid)) throw new Error("seal_grant_missing");
    var secret = b64ToBytes(grant.key_b64);
    var keys = await deriveKeysV1(secret);
    var plain = utf8(JSON.stringify(body));
    var pre = await preparePlainForAes(plain);
    var nonce = new Uint8Array(12);
    crypto.getRandomValues(nonce);
    var aesKey = await crypto.subtle.importKey("raw", keys.enc, { name: "AES-GCM" }, false, [
      "encrypt",
    ]);
    var ctBuf = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      aesKey,
      pre.bytes
    );
    var ct = new Uint8Array(ctBuf);
    var vt =
      (body.payload && body.payload.fields && body.payload.fields.visitor_terminal_id) ||
      global.__GR_VTID__ ||
      "vt_unknown";
    var env = {
      v: 1,
      alg: pre.alg,
      session_id: sid,
      visitor_terminal_id: String(vt || "vt_unknown"),
      batch_id: String((body && body.batch_id) || ""),
      nonce_b64: bytesToB64(nonce),
      ciphertext_b64: bytesToB64(ct),
      sig_b64: "",
      key_mode: "session",
      seal_exp_ms: grant.exp_ms || 0,
    };
    env.sig_b64 = await hmacSha256B64(keys.mac, [
      utf8("v1|"),
      utf8(env.session_id),
      utf8("|"),
      utf8(env.visitor_terminal_id),
      utf8("|"),
      utf8(env.batch_id),
      utf8("|"),
      utf8(env.nonce_b64),
      utf8("|"),
      utf8(env.ciphertext_b64),
    ]);
    return env;
  }

  async function sealIngestBody(body) {
    adoptFromGlobals();
    if (grantIsV2(grant)) {
      return sealIngestBodyV2(body);
    }
    // Production require sealed should not hit v1 — open always issues v2 when require_v2
    if (isRequireSealed() && !(global.__GR_SEAL_ALLOW_V1__ || global.__GR_FORCE_PLAIN_INGEST__)) {
      throw new Error("seal_v2_required");
    }
    return sealIngestBodyV1(body);
  }

  async function prepareUpload(apiBase, body) {
    var base = String(apiBase || "").replace(/\/$/, "");
    adoptFromGlobals();
    var need = isRequireSealed();
    var forcePlain = !!global.__GR_FORCE_PLAIN_INGEST__;
    var sid = (body && body.session_id) || "";
    if (forcePlain && !need) {
      return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
    }
    if (!need) {
      await waitForGrant(sid, 400);
      sid = resolveSealSessionId(sid);
      if (body && sid) body.session_id = sid;
      if (!grantValid(sid)) {
        return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
      }
    } else {
      var bid0 = String((body && body.batch_id) || "");
      var heavySeal =
        bid0.indexOf("B10") === 0 ||
        bid0.indexOf("B10x_") === 0 ||
        bid0 === "B0_bootstrap" ||
        bid0 === "B7_sandbox";
      await waitForGrant(sid, heavySeal ? 20000 : 16000);
      sid = resolveSealSessionId(sid);
      if (body && sid) body.session_id = sid;
      if (!grantValid(sid) && !grantRawValid()) throw new Error("seal_grant_unavailable");
      if (!grantValid(sid) && grantRawValid() && grant.session_id) {
        sid = String(grant.session_id);
        if (body) body.session_id = sid;
      }
      if (!grantValid(sid)) throw new Error("seal_grant_unavailable");
      // Preload WASM during wait window
      if (grantIsV2(grant)) {
        try {
          await ensureWasm(grant);
        } catch (eW) {
          throw eW;
        }
      }
    }
    try {
      var bid1 = String((body && body.batch_id) || "");
      var heavyCrypto =
        bid1.indexOf("B10x_") === 0 || bid1 === "B10_hw_curves" || bid1 === "B7_sandbox";
      var sealTimeoutMs = need ? (heavyCrypto ? 18000 : 12000) : 2500;
      var env = await Promise.race([
        sealIngestBody(body),
        new Promise(function (_, reject) {
          setTimeout(function () {
            reject(new Error("seal_timeout"));
          }, sealTimeoutMs);
        }),
      ]);
      return {
        url: base + "/v1/ingest/sealed",
        body: JSON.stringify(env),
        sealed: true,
        session_id: sid,
        seal_v: env.v,
      };
    } catch (e) {
      if (need || isRequireSealed()) throw e;
      return { url: base + "/v1/ingest", body: JSON.stringify(body), sealed: false };
    }
  }

  function preferKeepaliveOverBeacon() {
    return true;
  }

  adoptFromGlobals();

  global.GRSeal = {
    setGrant: setGrant,
    setRequireSealed: setRequireSealed,
    isRequireSealed: isRequireSealed,
    grantValid: grantValid,
    grantRawValid: grantRawValid,
    adoptFromGlobals: adoptFromGlobals,
    resolveSealSessionId: resolveSealSessionId,
    waitForGrant: waitForGrant,
    sealIngestBody: sealIngestBody,
    prepareUpload: prepareUpload,
    preferKeepaliveOverBeacon: preferKeepaliveOverBeacon,
    ensureWasm: ensureWasm,
    hasCompressionStream: function () {
      return typeof CompressionStream !== "undefined";
    },
    version: "gr_session_seal_v2",
    suite: SUITE_S2,
    wasmModuleId: WASM_ID,
  };
})(typeof window !== "undefined" ? window : globalThis);
