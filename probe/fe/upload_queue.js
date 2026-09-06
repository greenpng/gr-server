/**
 * GR UploadQueue — sub-pack self-managed upload (design from v57 upload_queue).
 * Priority sorts kick/upload order only; items never wait on each other to finish.
 */
(function (global) {
  "use strict";
  var cfg = {
    // iss/70 P2: start conservative; AIMD raises up to concurrency_max.
    concurrency: 3,
    concurrency_max: 6,
    concurrency_floor: 1,
    ramp_after_first: 4,
    /** Soft mid-ramp when B0 is only enqueued (not yet ok). */
    mid_ramp_concurrency: 4,
    apiBase: "",
    session_id: "",
    inject_path: "app",
    alive_retry_ms: 30000,
    /** Soft/mid default cap; hard anchors use lifecycle hard_max (see maxAttemptsForItem). */
    max_attempts_alive: 5,
    max_attempts_hide: 5,
    /** Hard commercial anchors (B10 / form-carrying lite) get extra priority on pagehide. */
    hard_anchor_batches: [
      "B10_hw_curves",
      "B0_bootstrap",
      "B2_hardware",
      "B3_system",
      "B1_conflict",
      "B12_anti_camouflage",
      "B8_gateway",
      "B8_gateway_early",
      // Silicon deepen: treat as pagehide-hard when residual completeness matters
      "B10x_silicon_ulp",
      "B10x_silicon_noderiv",
      "B10x_silicon_rint",
    ],
    hard_anchor_priority_boost: 1000,
    /** Extra boost after B10 lands so B10x outruns mid packs on short dwell. */
    b10x_post_b10_boost: 1500,
  };
  var pending = [];
  var inflight = 0;
  /** Items currently in postOne (for material-state / hard-SLA checks). */
  var inflightItems = [];
  /** Concurrent heavy (B10x/B7) uploads — cap 1 to reduce Failed to fetch. */
  var heavyInflight = 0;
  /** Counters for iss/70 P0/P1 observability (collect vs transport separation). */
  var transportRetryCount = 0;
  var captureFreezeCount = 0;
  var ackStored = 0;
  var ackDuplicate = 0;
  var ackMerged = 0;
  var ackConflict = 0;
  var aimdDowns = 0;
  var aimdUps = 0;
  /** Material Registry: batch_id → record (generation, hash, state, ack). */
  var materialRegistry = Object.create(null);
  /** Rolling success window for AIMD. */
  var recentUploadOk = 0;
  var recentUploadFail = 0;
  var sent = 0;
  var failed = 0;
  var skipped = 0;
  var hardFlushed = 0;
  var hardRetries = 0;
  var attempts = Object.create(null);
  var sentKeys = Object.create(null); // GA4-like: do not re-upload already-sent batch keys
  var flushReason = null;
  var hiding = false;
  var ramped = false;
  /** When set, identity/session uploads stop (cool / cycle_complete / 410). */
  var haltState = null; // { reason, session_id, code, at_ms }
  var haltedDrops = 0;
  /** In-flight AbortControllers — aborted on halt so browser does not complete more 410s. */
  var inflightCtrls = [];
  /** Session-level upload outcomes (P2 observability). */
  var sessionOutcome = {
    sealed_ok: 0,
    sealed_reject: 0,
    plain_ok: 0,
    network: 0,
    other_fail: 0,
    batches_ok: Object.create(null),
    /** Heartbeat/start markers only — must not block final payload. */
    batches_started: Object.create(null),
  };
  var outcomeReported = false;
  /** Counts soft/mid drops on pagehide and success short-circuits (ops). */
  var pagehideDropped = 0;
  var successShortCircuit = 0;
  var softAbortOnUnload = 0;
  /** force_recollect budget: max 1 per batch after terminal ok (iss/65). */
  var forceRecollectUsed = Object.create(null);

  /** Must-land silicon trio (EDH); deep is best-effort after these. */
  var B10X_MUST = ["B10x_silicon_ulp", "B10x_silicon_noderiv", "B10x_silicon_rint"];

  function batchAlreadyOk(batchId) {
    try {
      return !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok[String(batchId || "")]);
    } catch (e) {
      return false;
    }
  }

  /** Stable JSON for material hash (sorted object keys). */
  function stableStringify(v) {
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    if (Array.isArray(v)) {
      return "[" + v.map(stableStringify).join(",") + "]";
    }
    var keys = Object.keys(v).sort();
    var parts = [];
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i];
      parts.push(JSON.stringify(k) + ":" + stableStringify(v[k]));
    }
    return "{" + parts.join(",") + "}";
  }

  /** Sync SHA-256 hex (browser + node). Material identity only. */
  function sha256Hex(str) {
    try {
      if (typeof require === "function") {
        var c = require("crypto");
        if (c && c.createHash) return c.createHash("sha256").update(String(str), "utf8").digest("hex");
      }
    } catch (eN) {}
    // Minimal pure-js SHA-256 for browser lab (not crypto-grade multi-block edge; fine for dedupe).
    function rotr(n, x) {
      return (x >>> n) | (x << (32 - n));
    }
    var K = [
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
      0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
      0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
      0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
      0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
      0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
      0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
      0xc67178f2,
    ];
    function toBytes(s) {
      var utf8 = unescape(encodeURIComponent(String(s)));
      var arr = [];
      for (var i = 0; i < utf8.length; i++) arr.push(utf8.charCodeAt(i) & 255);
      return arr;
    }
    var bytes = toBytes(str);
    var l = bytes.length;
    var bitLen = l * 8;
    bytes.push(0x80);
    while ((bytes.length % 64) !== 56) bytes.push(0);
    for (var i = 7; i >= 0; i--) bytes.push((bitLen / Math.pow(2, i * 8)) & 255);
    var H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for (var off = 0; off < bytes.length; off += 64) {
      var w = new Array(64);
      for (var t = 0; t < 16; t++) {
        var j = off + t * 4;
        w[t] = (bytes[j] << 24) | (bytes[j + 1] << 16) | (bytes[j + 2] << 8) | bytes[j + 3];
      }
      for (t = 16; t < 64; t++) {
        var s0 = rotr(7, w[t - 15]) ^ rotr(18, w[t - 15]) ^ (w[t - 15] >>> 3);
        var s1 = rotr(17, w[t - 2]) ^ rotr(19, w[t - 2]) ^ (w[t - 2] >>> 10);
        w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0;
      }
      var a = H[0],
        b = H[1],
        c2 = H[2],
        d = H[3],
        e = H[4],
        f = H[5],
        g = H[6],
        h = H[7];
      for (t = 0; t < 64; t++) {
        var S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
        var ch = (e & f) ^ (~e & g);
        var t1 = (h + S1 + ch + K[t] + w[t]) | 0;
        var S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
        var maj = (a & b) ^ (a & c2) ^ (b & c2);
        var t2 = (S0 + maj) | 0;
        h = g;
        g = f;
        f = e;
        e = (d + t1) | 0;
        d = c2;
        c2 = b;
        b = a;
        a = (t1 + t2) | 0;
      }
      H[0] = (H[0] + a) | 0;
      H[1] = (H[1] + b) | 0;
      H[2] = (H[2] + c2) | 0;
      H[3] = (H[3] + d) | 0;
      H[4] = (H[4] + e) | 0;
      H[5] = (H[5] + f) | 0;
      H[6] = (H[6] + g) | 0;
      H[7] = (H[7] + h) | 0;
    }
    var out = "";
    for (i = 0; i < 8; i++) {
      var hx = (H[i] >>> 0).toString(16);
      out += ("00000000" + hx).slice(-8);
    }
    return out;
  }

  /**
   * iss/72: material hash = SHA-256 of canonical inner fields only.
   * Request-body SHA / attempt_id / seal envelope are NOT material identity.
   */
  function payloadHashOf(payload) {
    try {
      var fields = payload && payload.fields != null ? payload.fields : payload || {};
      var s = stableStringify(fields);
      return "sha256:" + sha256Hex(s);
    } catch (e) {
      return "sha256:0";
    }
  }

  /** logical key: session|batch|source|generation (iss/72) */
  function registryKey(sessionId, batchId, source, generation) {
    return (
      String(sessionId || cfg.session_id || "") +
      "|" +
      String(batchId || "") +
      "|" +
      String(source || "main") +
      "|" +
      String(generation != null ? generation : 1)
    );
  }

  function registryGet(batchId, source, sessionId, generation) {
    var sid = sessionId != null ? sessionId : cfg.session_id || "";
    var gen = generation != null ? generation : 1;
    return materialRegistry[registryKey(sid, batchId, source, gen)] || null;
  }

  function registryPut(rec) {
    if (!rec || !rec.batch_id) return;
    var sid = rec.session_id || cfg.session_id || "";
    var gen = rec.material_generation != null ? rec.material_generation : 1;
    materialRegistry[registryKey(sid, rec.batch_id, rec.source || "main", gen)] = rec;
  }

  /**
   * iss/70 Material Registry snapshot for a logical batch.
   * States: new|collected|queued|uploading|retry_wait|acked|conflict|transport_ok_unverified|absent
   */
  function registryUpsertFromItem(item, state) {
    if (!item || !item.batch_id) return null;
    var src = item.source || "main";
    var sid = item.session_id || cfg.session_id || "";
    var gen = item.material_generation != null ? item.material_generation : 1;
    var prev = registryGet(item.batch_id, src, sid, gen) || {};
    var rec = {
      batch_id: String(item.batch_id),
      source: src,
      session_id: sid,
      capture_id: item.capture_id || prev.capture_id || null,
      material_generation: gen,
      material_hash: item.payload_hash || prev.material_hash || null,
      payload_hash: item.payload_hash || prev.payload_hash || null,
      request_hash: prev.request_hash || null,
      state: state || prev.state || "collected",
      ack_type: prev.ack_type || null,
      transport_attempts: item._transport_attempts || prev.transport_attempts || 0,
      updated_ms: Date.now(),
    };
    registryPut(rec);
    return rec;
  }

  /**
   * Apply server ACK (iss/72 strict).
   * Only explicit ack.type in {stored,duplicate,merged} → ACKED.
   * Missing/empty ack → transport_ok_unverified (NOT batches_ok).
   */
  function applyAck(item, j) {
    j = j || {};
    var ack = j.ack && typeof j.ack === "object" ? j.ack : null;
    var type = ack && ack.type ? String(ack.type) : "";
    var bid = String((item && item.batch_id) || "");
    var src = (item && item.source) || "main";
    var sid = (item && item.session_id) || cfg.session_id || "";
    var gen =
      (ack && ack.material_generation != null
        ? ack.material_generation
        : item && item.material_generation != null
          ? item.material_generation
          : 1) | 0;
    // Prefer client material hash for registry identity; server body hash is request_hash.
    var materialHash =
      (ack && (ack.material_hash || ack.client_payload_hash)) ||
      (item && item.payload_hash) ||
      null;
    var requestHash = (ack && ack.payload_hash) || j.payload_hash || null;
    if (!type) {
      if (j.conflict === true || j.ack_conflict === true) type = "conflict";
      else if (j.duplicate === true || j.cold_skipped_unchanged === true) type = "duplicate";
      else if (j.merged === true) type = "merged";
      else if (j.ok === false && j.error) type = "rejected";
      else if (j.empty_body === true) type = "transport_ok_unverified";
      else if (j.ok === true && !ack) type = "transport_ok_unverified";
      else type = "transport_ok_unverified";
    }
    // Validate key match when server provides fields.
    // Explicit server type stored|duplicate|merged is authoritative (iss/72).
    // Do not reclassify server-success into conflict on minor echo mismatches
    // (sealed path may omit/reshape capture_id; material_hash remains client-side).
    var serverExplicitOk =
      type === "stored" || type === "duplicate" || type === "merged";
    if (ack && !serverExplicitOk) {
      if (ack.batch_id && String(ack.batch_id) !== bid) type = "rejected";
      if (ack.source && String(ack.source) !== src) type = "rejected";
      if (ack.session_id && String(ack.session_id) !== String(sid)) type = "rejected";
      if (
        ack.material_generation != null &&
        Number(ack.material_generation) !==
          Number(item && item.material_generation != null ? item.material_generation : 1)
      ) {
        type = "rejected";
      }
    }
    // Only force conflict when server says so (or explicit conflict flags).
    if (type !== "conflict" && (j.conflict === true || j.ack_conflict === true)) {
      type = "conflict";
    }
    var rec = registryGet(bid, src, sid, gen) || {
      batch_id: bid,
      source: src,
      session_id: sid,
      material_generation: gen,
    };
    rec.material_hash = materialHash || rec.material_hash;
    rec.payload_hash = materialHash || rec.payload_hash;
    rec.request_hash = requestHash || rec.request_hash;
    rec.capture_id = (item && item.capture_id) || rec.capture_id;
    var ingest = (j && j.ingest && typeof j.ingest === "object") ? j.ingest : {};
    var durability = String(
      (j && j.durability_state) ||
        (ack && ack.durability_state) ||
        ingest.durability_state ||
        ""
    );
    var coldWritten = j.cold_written;
    if (coldWritten === undefined) coldWritten = ingest.cold_written;
    var coldOk =
      coldWritten !== false ||
      j.cold_skipped_unchanged === true ||
      ingest.cold_skipped_unchanged === true ||
      j.same_capture === true ||
      ingest.same_capture === true ||
      durability === "duplicate_durable";
    if (
      durability === "stored_primary_only" ||
      durability === "accepted_pending_durability" ||
      durability === "failed"
    ) {
      coldOk = false;
    }
    var verified =
      (type === "stored" || type === "duplicate" || type === "merged") && coldOk;
    if (!coldOk && (type === "stored" || type === "duplicate" || type === "merged")) {
      type = "transport_ok_unverified";
    }
    rec.ack_type = type;
    rec.durability_state = durability || (coldOk ? "stored_durable" : "stored_primary_only");
    rec.updated_ms = Date.now();
    try {
      if (
        verified &&
        bid === "B11_interaction" &&
        global.GRRpaMonitor &&
        typeof global.GRRpaMonitor.ackSegment === "function"
      ) {
        global.GRRpaMonitor.ackSegment(
          src,
          (ack && ack.seq_end) || item.rpa_seq_end,
          (ack && ack.segment_id) || item.rpa_segment_id
        );
      }
    } catch (eRpaAck) {}
    if (verified) {
      rec.state = "acked";
      ackStored += type === "stored" ? 1 : 0;
      if (type === "duplicate") ackDuplicate++;
      if (type === "merged") ackMerged++;
    } else if (type === "conflict") {
      rec.state = "conflict";
      ackConflict++;
    } else if (type === "transport_ok_unverified") {
      rec.state = "transport_ok_unverified";
    } else {
      rec.state = "rejected";
    }
    registryPut(rec);
    return { type: type, verified: verified, material_hash: materialHash, request_hash: requestHash };
  }

  /** Deep-clone JSON-safe material so post-freeze collector churn cannot alias nested fields. */
  function deepCloneJson(v) {
    try {
      return JSON.parse(JSON.stringify(v));
    } catch (eDc) {
      return v;
    }
  }

  /**
   * iss/72: freeze capture at boundary — ALL material fields before hash.
   * After freeze, postOne must not mutate item.payload.
   * iss/75: deep-clone fields (shallow Object.assign left nested refs shared with collectors
   * → false `capture_mutated_blocked` on B0/B1/B10 enrich / dual-land).
   */
  function freezeCapture(item) {
    if (!item || item._capture_frozen) return item;
    try {
      var productVer =
        (global.__GR_PRODUCT_VERSION__ ||
          (global.__GR_BOOT__ &&
            (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
          "") + "";
      var payload = item.payload;
      if (!payload || typeof payload !== "object") payload = {};
      else payload = Object.assign({}, payload);
      var pf =
        payload.fields && typeof payload.fields === "object"
          ? deepCloneJson(payload.fields)
          : {};
      if (!pf || typeof pf !== "object") pf = {};
      if (productVer && !pf.product_version) pf.product_version = productVer;
      if (productVer && !payload.product_version) payload.product_version = productVer;
      // Timing once
      try {
        if (pf.t_perf == null) {
          var tPerf =
            typeof performance !== "undefined" && performance.now ? performance.now() : null;
          var tWall = Date.now();
          if (tPerf != null && isFinite(tPerf)) {
            pf.t_perf = Math.round(tPerf * 1000) / 1000;
            pf.t_wall_ms = tWall;
          }
        }
      } catch (eClk) {}
      try {
        if (global.__GR_COMPUTE_PRESSURE_STATE__ && !pf.compute_pressure_state) {
          pf.compute_pressure_state = String(global.__GR_COMPUTE_PRESSURE_STATE__);
        }
      } catch (ePr) {}
      // Cohort / FE impl — frozen INTO material before hash (iss/72 P0-3).
      try {
        if (!pf.cohort_color_gamut && typeof matchMedia === "function") {
          if (matchMedia("(color-gamut: rec2020)").matches) pf.cohort_color_gamut = "rec2020";
          else if (matchMedia("(color-gamut: p3)").matches) pf.cohort_color_gamut = "p3";
          else if (matchMedia("(color-gamut: srgb)").matches) pf.cohort_color_gamut = "srgb";
          else pf.cohort_color_gamut = "unknown";
        }
        if (pf.cohort_dpr_bucket == null && typeof devicePixelRatio === "number") {
          pf.cohort_dpr_bucket = Math.round(devicePixelRatio * 4) / 4;
        }
        if (!pf.cohort_pointer && typeof matchMedia === "function") {
          pf.cohort_pointer = matchMedia("(pointer: fine)").matches
            ? "fine"
            : matchMedia("(pointer: coarse)").matches
              ? "coarse"
              : "none";
        }
        if (!pf.cohort_intl_locale) {
          try {
            pf.cohort_intl_locale = Intl.DateTimeFormat().resolvedOptions().locale || "";
            pf.cohort_intl_calendar = Intl.DateTimeFormat().resolvedOptions().calendar || "";
            pf.cohort_intl_numbering = Intl.DateTimeFormat().resolvedOptions().numberingSystem || "";
          } catch (eIntl) {}
        }
        if (pf.cohort_speech_voices_n == null) {
          try {
            if (typeof speechSynthesis !== "undefined" && speechSynthesis.getVoices) {
              pf.cohort_speech_voices_n = (speechSynthesis.getVoices() || []).length;
            }
          } catch (eVo) {}
        }
        var fePacks = String(global.__GR_FE_PACKS_VERSION__ || global.__GR_FE_CODE_VERSION__ || "");
        if (fePacks) {
          if (!pf.fe_packs_version) pf.fe_packs_version = fePacks;
          if (!pf.fe_code_version) pf.fe_code_version = fePacks;
        }
        // Seal B10 requires fe_impl_version ∈ product epoch allowlist.
        // Prefer server product over sticky module BUILD_IMPL (v5.8.*) stamps.
        try {
          var prodEpoch =
            String(
              global.__GR_SERVER_PRODUCT_VERSION__ ||
                global.__GR_PRODUCT_VERSION__ ||
                (global.__GR_BOOT__ &&
                  (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
                ""
            ) || "";
          if (prodEpoch) {
            pf.fe_impl_version = prodEpoch;
            try {
              global.__GR_FE_IMPL_VERSION__ = prodEpoch;
            } catch (eSet) {}
          } else if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        } catch (eImpl) {
          if (!pf.fe_impl_version && global.__GR_FE_IMPL_VERSION__) {
            pf.fe_impl_version = String(global.__GR_FE_IMPL_VERSION__);
          }
        }
        if (!pf.fe_build_impl && global.__GR_BUILD_IMPL__) {
          pf.fe_build_impl = String(global.__GR_BUILD_IMPL__);
        }
      } catch (eCoh) {}
      payload.fields = pf;
      item.payload = payload;
      if (!item.capture_id) {
        item.capture_id =
          "cap_" +
          String(item.batch_id || "b") +
          "_" +
          String(Date.now()) +
          "_" +
          String(Math.floor(Math.random() * 1e6));
      }
      if (item.material_generation == null) item.material_generation = 1;
      item.collected_at_ms = item.collected_at_ms || Date.now();
      item.payload_hash = payloadHashOf(payload);
      item.material_hash = item.payload_hash;
      item._capture_frozen = true;
      // Deep-freeze snapshot for retry identity checks + restore on post-freeze mutation.
      try {
        item._frozen_fields = deepCloneJson(payload.fields || {});
        item._frozen_payload_json = stableStringify(item._frozen_fields || {});
      } catch (eSnap) {}
      captureFreezeCount++;
      registryUpsertFromItem(item, "collected");
    } catch (eFz) {
      try {
        item._capture_frozen = true;
      } catch (e2) {}
    }
    return item;
  }

  /** Reset queue/registry state when session_id changes (iss/72 P1-2). */
  function resetSessionState(reason) {
    materialRegistry = Object.create(null);
    sessionOutcome = {
      sealed_ok: 0,
      sealed_reject: 0,
      plain_ok: 0,
      network: 0,
      other_fail: 0,
      batches_ok: Object.create(null),
      batches_started: Object.create(null),
    };
    forceRecollectUsed = Object.create(null);
    sentKeys = Object.create(null);
    attempts = Object.create(null);
    outcomeReported = false;
    try {
      global.__GR_RECEIVED_BATCHES__ = [];
    } catch (eR) {}
    // Drop pending from other sessions
    try {
      var keep = [];
      for (var i = 0; i < pending.length; i++) {
        if (pending[i] && String(pending[i].session_id || "") === String(cfg.session_id || "")) {
          keep.push(pending[i]);
        }
      }
      pending = keep;
    } catch (eP) {}
    try {
      if (global.GROps && GROps.report) {
        GROps.report(
          "queue_session_reset",
          "upload",
          { reason: String(reason || "session_change") },
          "info"
        );
      }
    } catch (eO) {}
  }

  /**
   * iss/70 P2: effective upload concurrency = min of budgets (never max-stack).
   */
  function effectiveUploadCap() {
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var hardMax = Math.max(floor, Number(cfg.concurrency_max) || 6);
    var base = Math.max(floor, Number(cfg.concurrency) || 3);
    // Lifecycle: unloading → 1–2; background → low.
    var lifeCap = hardMax;
    try {
      if (isPageUnloading() || hiding || global.__GR_PAGE_UNLOADING__) lifeCap = 2;
      else if (global.__GR_PAGE_BACKGROUNDED__ || isPageBackgrounded()) lifeCap = 2;
    } catch (eL) {}
    // Pressure: serious/critical → floor.
    try {
      var st = String(global.__GR_COMPUTE_PRESSURE_STATE__ || "").toLowerCase();
      if (st === "serious" || st === "critical") lifeCap = Math.min(lifeCap, floor);
    } catch (eP) {}
    // Fail storm: half.
    if (recentUploadFail >= 3 && recentUploadOk < recentUploadFail) {
      lifeCap = Math.min(lifeCap, Math.max(floor, Math.floor(base / 2) || floor));
    }
    return Math.max(floor, Math.min(hardMax, base, lifeCap));
  }

  /** AIMD: success window +1 (cap max); fail → half. */
  function aimdOnSuccess() {
    recentUploadOk++;
    recentUploadFail = Math.max(0, recentUploadFail - 1);
    var maxC = Math.max(1, Number(cfg.concurrency_max) || 6);
    if (recentUploadOk >= 3 && recentUploadFail === 0) {
      var next = Math.min(maxC, (Number(cfg.concurrency) || 3) + 1);
      if (next > cfg.concurrency) {
        cfg.concurrency = next;
        aimdUps++;
      }
      recentUploadOk = 0;
    }
  }

  function aimdOnFail() {
    recentUploadFail++;
    recentUploadOk = 0;
    var floor = Math.max(1, Number(cfg.concurrency_floor) || 1);
    var cur = Number(cfg.concurrency) || 3;
    var next = Math.max(floor, Math.floor(cur / 2) || floor);
    if (next < cur) {
      cfg.concurrency = next;
      aimdDowns++;
    }
  }

  /** True if this batch has a capture in pending/retry/upload (not yet batches_ok). */
  function hasPendingCapture(batchId, sessionId) {
    var bid = String(batchId || "");
    if (!bid) return false;
    var sid = sessionId != null ? String(sessionId) : "";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      return true;
    }
    var i;
    for (i = 0; i < pending.length; i++) if (match(pending[i])) return true;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return true;
    return false;
  }

  /**
   * Material/transport state for hard-SLA, registry, and tests.
   * acked | conflict | uploading | retry_wait | queued | collected | absent
   */
  function materialState(batchId, sessionId, source, generation) {
    var bid = String(batchId || "");
    if (!bid) return "absent";
    if (batchAlreadyOk(bid)) return "acked";
    var sid = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
    var src = source || "main";
    var gen = generation != null ? generation : 1;
    var rec = registryGet(bid, src, sid, gen);
    if (rec && rec.state === "acked") return "acked";
    if (rec && rec.state === "conflict") return "conflict";
    function match(it) {
      if (!it || String(it.batch_id || "") !== bid) return false;
      if (sid && it.session_id && String(it.session_id) !== sid) return false;
      if (src && it.source && String(it.source) !== String(src)) return false;
      return true;
    }
    var i;
    for (i = 0; i < inflightItems.length; i++) if (match(inflightItems[i])) return "uploading";
    for (i = 0; i < pending.length; i++) {
      if (!match(pending[i])) continue;
      if (pending[i]._retry_after_ms && pending[i]._retry_after_ms > Date.now()) return "retry_wait";
      return "queued";
    }
    if (rec && rec.state) return rec.state;
    return "absent";
  }

  function removeInflightItem(item) {
    var kWant = "";
    try {
      kWant = keyOf(item);
    } catch (eK) {
      kWant = "";
    }
    var removed = 0;
    for (var i = inflightItems.length - 1; i >= 0; i--) {
      var cur = inflightItems[i];
      if (cur === item) {
        inflightItems.splice(i, 1);
        removed++;
        continue;
      }
      // Identity can diverge after freeze/clone — also drop by material key.
      try {
        if (kWant && keyOf(cur) === kWant) {
          inflightItems.splice(i, 1);
          removed++;
        }
      } catch (e2) {}
    }
    // Own the counter: callers must not also inflight-- after this (was double-
    // decrementing → negative inflight under multi-tab / multi-key prune).
    if (removed > 0) inflight = Math.max(0, inflight - removed);
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (inflight < 0) inflight = 0;
  }

  /** Drop stuck inflight slots so multi-tab never accumulates zombie transport rows. */
  function pruneStaleInflight() {
    var cap = Math.max(Number(cfg.concurrency_max) || 6, Number(cfg.concurrency) || 3) * 2;
    if (inflightItems.length <= cap && inflight <= cap) return;
    // Hard clamp: keep newest cap items, drop older zombies.
    if (inflightItems.length > cap) {
      inflightItems = inflightItems.slice(-cap);
    }
    if (inflight > inflightItems.length) inflight = inflightItems.length;
    if (heavyInflight > 1) heavyInflight = 1;
  }

  function isB10xMustLand(batchId) {
    var id = String(batchId || "");
    for (var i = 0; i < B10X_MUST.length; i++) if (B10X_MUST[i] === id) return true;
    return false;
  }

  /**
   * pagehide / unload allowlist — only mint-critical + secondary silicon.
   * Mid/dense/R/B11 are dropped so keepalive window is not wasted (iss/65).
   */
  function isPagehideAllowlisted(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    if (id.indexOf("B10x_silicon_") === 0) {
      // On unload: prefer must-land; deep only if must-land already ok or nothing else pending.
      if (id === "B10x_silicon_deep") {
        if (!isPageUnloading()) return true;
        var mustPending = false;
        for (var i = 0; i < B10X_MUST.length; i++) {
          if (!batchAlreadyOk(B10X_MUST[i])) {
            // still need must — allow deep only if already inflight material not required
            mustPending = true;
            break;
          }
        }
        // If must still missing, drop deep on pagehide (save slots for ulp/rint/noderiv).
        return !mustPending;
      }
      return true;
    }
    if (
      id === "B0_bootstrap" ||
      id === "B2_hardware" ||
      id === "B3_system" ||
      id === "B1_conflict" ||
      id === "B12_anti_camouflage" ||
      id === "B8_gateway" ||
      id === "B8_gateway_early" ||
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B7_sandbox"
    ) {
      return true;
    }
    return false;
  }

  /**
   * Pagehide / flush priority: commercial hard + secondary silicon (B10x/B47/B18).
   * NOTE: do NOT use this alone for ops severity — B10x/B47 network exhaust must
   * report as deepen/soft exhausted (warn), not upload_hard_exhausted (error).
   * Prod v150: B10x_deep / B47 mis-tagged as hard_exhausted on Failed to fetch.
   */
  function isHardAnchorBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    // Secondary silicon/infra: hard *priority* on pagehide only (not always-hard).
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    ) {
      return isPageUnloading() || !!sessionOutcome.batches_ok["B10_hw_curves"];
    }
    // B10x: hard on pagehide / when B10 already ok (short-visit residual path)
    if (id.indexOf("B10x_silicon_") === 0) {
      if (isPageUnloading()) return true;
      try {
        if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) return true;
      } catch (eB) {}
      return false;
    }
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    // alias mid.curves / primary silicon residual only
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    return false;
  }

  /**
   * Ops severity gate: only true commercial anchors → upload_hard_exhausted error.
   * Silicon deepen / SAB / WebGPU network storms → deepen_exhausted warn.
   */
  function isCommercialHardForOps(batchId) {
    var id = String(batchId || "");
    if (!id) return false;
    if (id.indexOf("B10x_") === 0) return false;
    if (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B15_cross_curves"
    ) {
      return false;
    }
    if (id === "mid.curves" || id === "B10_hw_curves") return true;
    var list = cfg.hard_anchor_batches || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] === id) return true;
    }
    return false;
  }

  function isDeepenBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isDeepenBatch) {
        return !!GRProbeLifecycle.isDeepenBatch(batchId);
      }
    } catch (eD) {}
    var id = String(batchId || "");
    return id.indexOf("B10x_") === 0 || id === "B15_cross_curves";
  }

  function isHeavyBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isHeavyBatch) {
        return !!GRProbeLifecycle.isHeavyBatch(batchId);
      }
    } catch (eH) {}
    var id = String(batchId || "");
    return (
      isDeepenBatch(id) ||
      id === "B7_sandbox" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep"
    );
  }

  function heavyMaxInflight() {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.heavyMaxInflight) {
        return Math.max(1, GRProbeLifecycle.heavyMaxInflight() | 0);
      }
      if (global.GRProbeLifecycle && GRProbeLifecycle.POLICY) {
        return Math.max(1, (GRProbeLifecycle.POLICY.heavy_max_inflight || 1) | 0);
      }
    } catch (eM) {}
    return 1;
  }

  function isStartHeartbeatItem(item) {
    try {
      var f =
        (item && item.payload && item.payload.fields) ||
        (item && item.fields) ||
        null;
      if (!f || typeof f !== "object") return false;
      if (String(f.b10x_phase || "") === "start") return true;
      if (String(f.b10x_err || "") === "started") return true;
      if (String(f.b10x_err || "") === "helpers_missing") return true;
      return false;
    } catch (e) {
      return false;
    }
  }

  function noteOutcome(kind, batchId, item) {
    try {
      if (kind === "sealed_ok") sessionOutcome.sealed_ok++;
      else if (kind === "sealed_reject") sessionOutcome.sealed_reject++;
      else if (kind === "plain_ok") sessionOutcome.plain_ok++;
      else if (kind === "network") sessionOutcome.network++;
      else sessionOutcome.other_fail++;
      if (batchId && (kind === "sealed_ok" || kind === "plain_ok")) {
        var bid = String(batchId);
        // Start/heartbeat must not count as terminal success (blocks final multipath).
        if (isStartHeartbeatItem(item)) {
          sessionOutcome.batches_started[bid] = 1;
          return;
        }
        sessionOutcome.batches_ok[bid] = 1;
        // After B10 lands, boost pending B10x so residual path wins short dwell.
        if (bid === "B10_hw_curves") {
          try {
            boostPendingB10x();
          } catch (eBoost) {}
        }
      }
    } catch (eO) {}
  }

  function boostPendingB10x() {
    var boost = Number(cfg.b10x_post_b10_boost || 1500);
    for (var i = 0; i < pending.length; i++) {
      var it = pending[i];
      if (!it) continue;
      var bid = String(it.batch_id || "");
      if (bid.indexOf("B10x_silicon_") === 0) {
        it.priority = Math.max(Number(it.priority || 0), boost);
      }
    }
    try {
      pending.sort(function (a, b) {
        return Number(b.priority || 0) - Number(a.priority || 0);
      });
    } catch (eSrt) {}
    pump();
  }

  /** P2: one session summary for ops (gateway_only vs browser_probe). */
  function reportSessionUploadSummary(force) {
    if (outcomeReported && !force) return;
    try {
      var okKeys = Object.keys(sessionOutcome.batches_ok || {});
      var hasB0 = !!sessionOutcome.batches_ok["B0_bootstrap"];
      var hasB10 = !!sessionOutcome.batches_ok["B10_hw_curves"];
      var hasB8 =
        !!sessionOutcome.batches_ok["B8_gateway"] ||
        !!sessionOutcome.batches_ok["B8_gateway_early"];
      var hasB10x = false;
      for (var k = 0; k < okKeys.length; k++) {
        if (String(okKeys[k]).indexOf("B10x_silicon_") === 0) {
          hasB10x = true;
          break;
        }
      }
      var onlyGw = hasB8 && !hasB0 && okKeys.length <= 2;
      var depthClass = onlyGw
        ? "gateway_only"
        : hasB0 && hasB10 && hasB10x
          ? "browser_b0_b10_silicon"
          : hasB0 && hasB10
            ? "browser_b0_b10"
            : hasB0
              ? "browser_partial"
              : okKeys.length
                ? "browser_lite"
                : "empty";
      if (global.GROps && GROps.report) {
        outcomeReported = true;
        GROps.report(
          "upload_session_summary",
          "upload",
          {
            sealed_ok: sessionOutcome.sealed_ok,
            sealed_reject: sessionOutcome.sealed_reject,
            plain_ok: sessionOutcome.plain_ok,
            network: sessionOutcome.network,
            other_fail: sessionOutcome.other_fail,
            batches_ok_n: okKeys.length,
            has_b0: hasB0,
            has_b10: hasB10,
            has_b10x: hasB10x,
            probe_depth_class: depthClass,
          },
          "info"
        );
      }
    } catch (eS) {}
  }

  /** True only on real unload — tab background must NOT set this. */
  function isPageUnloading() {
    try {
      return !!(global.__GR_PAGE_HIDING__ || global.__GR_PAGE_UNLOADING__);
    } catch (e) {
      return false;
    }
  }

  function isPageBackgrounded() {
    try {
      return !!global.__GR_PAGE_BACKGROUNDED__;
    } catch (e) {
      return false;
    }
  }

  function effectivePriority(item) {
    var p = item.priority || 0;
    var bid = String((item && item.batch_id) || "");
    // Always prefer commercial land + early caps over soft-pack storms (iss/75 texture race).
    if (bid === "B10_hw_curves" || bid === "mid.curves") p += 2200;
    else if (bid === "B2_hardware") p += 1800;
    else if (isCommercialLandFast(bid)) p += 1600;
    if (hiding && isHardAnchorBatch(bid)) {
      p += cfg.hard_anchor_priority_boost || 1000;
    }
    // Active maximize path (alive dwell): keep secondary silicon/infra + B10x above soft
    // media force storms (B19 dense hedge / client_hints). Aligns with probe_field_priority
    // silicon_p0 → media_p1/infra_p1 without blocking soft packs from eventually uploading.
    // Observed: Brave matrix dwell ended pending=250, B19×35, missing B47/B18/B46.
    if (!hiding && !isPageUnloading()) {
      var b10Ok = false;
      try {
        b10Ok = !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]);
      } catch (eB10) {}
      if (isB10xBatch(bid)) {
        p += b10Ok ? cfg.b10x_post_b10_boost || 1500 : 900;
      } else if (isSecondaryInfraBatch(bid)) {
        // B47/B18/B46/B10x_deep: must not sit behind soft force re-queues mid-dwell.
        p += b10Ok ? 1200 : 700;
      }
    }
    // Unload: must-land B10x above deep / secondary so short keepalive lands silicon.
    if (hiding || isPageUnloading()) {
      if (isB10xMustLand(bid)) p += 2500;
      else if (bid === "B10_hw_curves" || bid === "mid.curves") p += 3000;
      else if (bid === "B10x_silicon_deep") p -= 400;
      else if (!isPagehideAllowlisted(bid)) p -= 5000;
    }
    return p;
  }

  /**
   * Insert or replace pending by session|batch|source key.
   * Force items MUST coalesce (was: force=true skipped replace → N copies of B19/secondary).
   * Does not change wire semantics: still one transport path per key at a time.
   * @returns {"pushed"|"replaced"|"dropped"}
   */
  function queuePending(item) {
    if (!item || !item.batch_id) return "dropped";
    var k = keyOf(item);
    var genNew = (item.material_generation || 1) | 0;
    for (var i = 0; i < pending.length; i++) {
      if (keyOf(pending[i]) !== k) continue;
      var cur = pending[i];
      var genCur = (cur.material_generation || 1) | 0;
      // Never demote a higher material generation already queued.
      if (genNew < genCur) {
        skipped++;
        return "dropped";
      }
      // Same key: replace with fresher capture (force or not). Counts as dedupe.
      pending[i] = item;
      skipped++;
      return "replaced";
    }
    pending.push(item);
    return "pushed";
  }

  function keyOf(item) {
    return [item.session_id || cfg.session_id, item.batch_id, item.source || "main"].join("|");
  }

  function sessionOf(item) {
    return String(
      (item && item.session_id) || cfg.session_id || global.__GR_SESSION_ID__ || ""
    );
  }

  /** Batches that must stop when cycle is complete / cool (identity path). */
  function isIdentityBatch(batchId) {
    var id = String(batchId || "");
    if (!id) return true;
    // Page RPA after cool still uses B11 in some designs — block on hard halt too
    // because server require_active_session rejects ALL ingest on complete cycle.
    return true;
  }

  /**
   * Terminal = this cycle must stop identity uploads.
   * Align with BE:
   *   - 410 / cycle_closed / halt_uploads / analysis_terminal / cycle_complete → halt
   *   - soft probe_complete alone → NOT halt (still may upload rest packs)
   *   - open cool skip_identity → halt identity
   */
  /**
   * Schedule-final complete (maximize probe policy v5.8.53+).
   * True only when brain schedule is done AND silicon/B10 materials exist.
   * commercial_identity_final / dh_+curves alone is a milestone — NOT final.
   */
  function hardFinalComplete(j, cps) {
    cps = cps || {};
    var cov = cps.identity_coverage || j.identity_coverage || {};
    // EDH: block cool while silicon B10x still required/missing.
    try {
      var edh = j.edh || j.device_hypothesis || {};
      var rg = edh.research_gate || {};
      if (rg.b10x_required === true && rg.b10x_complete === false) return false;
      if (j.b10x_must_land === true) return false;
      var miss = (j.route_plan && j.route_plan.b10x_missing) || (j.b10x_missing) || [];
      if (miss && miss.length) return false;
      var rp = j.route_plan || (j.brain && j.brain.route_plan) || {};
      var packs = rp.packs || [];
      for (var pi = 0; pi < packs.length; pi++) {
        var pid = String((packs[pi] && (packs[pi].pack_id || packs[pi].id)) || "");
        if (pid.indexOf("B10x_silicon_") === 0) return false;
      }
    } catch (eEdh) {}

    // Primary residual B10 MUST land before any cool/final — do not trust digest alone.
    var ids = cps.received_batch_ids || j.received_batch_ids || [];
    var hasB10 = false;
    for (var i = 0; i < ids.length; i++) {
      var id = typeof ids[i] === "string" ? ids[i] : ids[i] && ids[i].batch_id;
      if (id === "B10_hw_curves" || id === "mid.curves") {
        hasB10 = true;
        break;
      }
    }
    try {
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) hasB10 = true;
      if (sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]) hasB10 = true;
    } catch (eLoc) {}
    try {
      if (j.b10_present === true && (sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"])) {
        hasB10 = true;
      }
    } catch (eB) {}
    // Without primary B10, never final (keeps self-heal / upload alive).
    if (!hasB10) return false;

    // commercial_identity_final alone → NOT final (continue soft/mid packs).
    var coverageComplete =
      cov.coverage_complete === true ||
      cps.coverage_complete === true ||
      j.coverage_complete === true ||
      (j.coverage && j.coverage.coverage_complete === true) ||
      (j.brain && j.brain.coverage && j.brain.coverage.coverage_complete === true);
    var stopProbe =
      j.stop_probe === true ||
      (j.route_plan && j.route_plan.stop_probe === true) ||
      (cps.probe_complete === true && coverageComplete);
    var brainTerm =
      j.analysis_terminal === true ||
      cps.analysis_terminal === true ||
      (j.analysis && j.analysis.analysis_terminal === true) ||
      (stopProbe && coverageComplete);
    var closed =
      cps.cycle_status === "complete" ||
      j.cycle_complete === true ||
      cps.cycle_closed === true ||
      j.cycle_closes === true ||
      (j.analysis && (j.analysis.cycle_closes === true || j.analysis.cycle_complete === true));
    var scheduleFinal =
      cov.brain_schedule_final === true ||
      cps.brain_schedule_final === true ||
      j.brain_schedule_final === true ||
      cov.final_analysis_ok === true ||
      cps.final_analysis_ok === true ||
      j.final_analysis_ok === true;

    // Final = primary B10 landed AND (schedule done OR closed OR hard_complete).
    if (scheduleFinal) return true;
    if (brainTerm || closed) return true;
    if (cov.hard_complete === true) return true;
    return false;
  }

  function parseTerminal(status, j) {
    j = j || {};
    var err = j.error || j.code || j.expired_reason || "";
    var code = j.code || j.expired_reason || "";
    var cps = j.cycle_probe_status || {};
    var analysis = j.analysis || {};
    var s =
      String(err) +
      " " +
      String(code) +
      " " +
      String(cps.cycle_status || "") +
      " " +
      String(cps.business_state || "") +
      " " +
      String(cps.expired_reason || "");

    // Soft cycle close (v5.8.124): HTTP 200 + accepted:false + halt_uploads.
    // Preferred over 410 so browser Network is not full of red errors.
    if (
      status === 200 &&
      j &&
      (j.accepted === false ||
        j.identity_accepted === false ||
        (j.halt_uploads === true &&
          (j.cycle_closed === true ||
            j.code === "cycle_complete" ||
            j.code === "cycle_purged" ||
            j.code === "incomplete_ttl" ||
            j.code === "session_expired" ||
            (j.cycle_probe_status && j.cycle_probe_status.http_soft_close))))
    ) {
      return {
        terminal: true,
        code: code || j.code || cps.expired_reason || "cycle_complete",
        status: 200,
        business_state: j.business_state || cps.business_state || "identity_complete_cool",
        soft_close: true,
      };
    }

    // Legacy HTTP 410: cycle no longer accepts identity ingest (complete|purged|ttl).
    if (status === 410) {
      return {
        terminal: true,
        code: code || cps.expired_reason || "session_expired",
        status: 410,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }

    // Authoritative hard+final complete (probe completeness + final analysis).
    if (hardFinalComplete(j, cps)) {
      return {
        terminal: true,
        code: code || "identity_final_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // halt_uploads from server only if hard complete OR cycle truly closed with hard materials.
    if (j.halt_uploads === true || cps.halt_uploads === true || analysis.halt_uploads === true) {
      if (!hardFinalComplete(j, cps) && cps.cycle_status !== "purged" && status !== 410) {
        // Thin halt flag — keep uploading B10 (server may lag; hard SLA continues).
        return { terminal: false };
      }
      var haltCode =
        code ||
        (analysis.cycle_complete || j.cycle_complete
          ? "cycle_complete"
          : cps.expired_reason || cps.cycle_status || cps.business_state || "halt_uploads");
      return {
        terminal: true,
        code: haltCode,
        status: status || 200,
        business_state: cps.business_state || "halt_uploads",
      };
    }

    // Cycle closed flags — only halt when hard materials present (or purged).
    if (
      j.cycle_closes === true ||
      j.cycle_complete ||
      cps.cycle_closed === true ||
      cps.cycle_status === "complete" ||
      cps.cycle_status === "purged" ||
      analysis.cycle_complete ||
      analysis.cycle_closes === true
    ) {
      if (cps.cycle_status === "purged") {
        return {
          terminal: true,
          code: "cycle_purged",
          status: status || 200,
          business_state: "purged",
        };
      }
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cycle_complete",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    // Soft analysis_terminal without B10: keep probing (do not halt).
    if (
      j.analysis_terminal === true ||
      analysis.analysis_terminal === true ||
      cps.analysis_terminal === true
    ) {
      if (!hardFinalComplete(j, cps)) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: "analysis_terminal",
        status: status || 200,
        business_state: "analysis_terminal",
      };
    }

    // Cool / open skip identity — ONLY when phase/business_state says cool AND silicon ok.
    if (
      j.phase === "cool" ||
      cps.business_state === "identity_complete_cool" ||
      (j.skip_identity_probe === true &&
        (j.phase === "cool" ||
          cps.cycle_status === "complete" ||
          cps.halt_uploads === true ||
          j.halt_uploads === true)) ||
      (j.skip_session_probe === true &&
        (j.phase === "cool" || cps.business_state === "identity_complete_cool"))
    ) {
      if (j.cool_silicon_ok === false || cps.cool_silicon_ok === false) {
        return { terminal: false };
      }
      // Prefer hard materials when available; thin cool already blocked server-side.
      if (!hardFinalComplete(j, cps) && j.cool_silicon_ok !== true && cps.has_b10 !== true) {
        return { terminal: false };
      }
      return {
        terminal: true,
        code: code || "cool",
        status: status || 200,
        business_state: "identity_complete_cool",
      };
    }

    if (/session_expired|cycle_purged|incomplete_ttl/i.test(s)) {
      return {
        terminal: true,
        code: code || "session_expired",
        status: status || 0,
        business_state: cps.business_state || code || "cycle_closed",
      };
    }
    // cycle_complete / halt_uploads in error string alone must not stop thin uploads.
    return { terminal: false };
  }

  function maybeLocalIdentityDone() {
    // Local channel only: FE-side upload progress. Does NOT close the cycle —
    // server analysis_terminal / complete_cycle / 410 remain authoritative for halt.
    var need = cfg.hard_anchor_batches || [];
    var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
    var ok = 0;
    var present = [];
    for (var i = 0; i < need.length; i++) {
      var bid = need[i];
      if (bid === "B8_gateway_early") continue;
      var k1 = [sid, bid, "main"].join("|");
      var k2 = bid === "B8_gateway" ? [sid, "B8_gateway_early", "main"].join("|") : null;
      if (sentKeys[k1] || (k2 && sentKeys[k2])) {
        ok++;
        present.push(bid);
      }
    }
    var idle = pending.length === 0 && inflight <= 1;
    // B0+B1+B2+B3+B12+B8 ≈ 6 without B10 is enough for "lite wave uploaded" local signal
    if (ok >= 5 && idle) {
      try {
        global.__GR_IDENTITY_UPLOADS_DONE__ = {
          at_ms: Date.now(),
          hard_sent: ok,
          present: present,
          session_id: sid,
          // Soft local progress — not cycle closed
          local_only: true,
          cycle_closed: !!(haltState || global.__GR_CYCLE_CLOSED__),
        };
        global.dispatchEvent(
          new CustomEvent("gr-identity-uploads-done", {
            detail: global.__GR_IDENTITY_UPLOADS_DONE__,
          })
        );
      } catch (eD) {}
    }
    // Queue fully idle: notify multi-tick / observers (not halt).
    if (pending.length === 0 && inflight === 0) {
      try {
        global.__GR_UPLOAD_QUEUE_IDLE__ = {
          at_ms: Date.now(),
          sent: sent,
          session_id: sid,
          halted: !!haltState,
        };
        global.dispatchEvent(
          new CustomEvent("gr-upload-queue-idle", {
            detail: global.__GR_UPLOAD_QUEUE_IDLE__,
          })
        );
      } catch (eI) {}
    }
  }

  function phaseForCode(code, reason) {
    var c = String(code || reason || "");
    if (/incomplete_ttl|purged|cycle_purged/i.test(c)) return "expired";
    if (/cool|cycle_complete|analysis_terminal|halt|probe_complete|stop_probe/i.test(c)) return "cool";
    return "cool";
  }

  function applyHalt(reason, sessionId, code) {
    var sid = String(sessionId || cfg.session_id || global.__GR_SESSION_ID__ || "");
    var c = code || reason || "halt";
    // Idempotent: same session already halted — still abort leftovers.
    try {
      reportSessionUploadSummary(true);
    } catch (eSum) {}
    var cLow0 = String(c || reason || "").toLowerCase();
    // Soft-close preferred: cycle complete / cool are product-normal (HTTP 200), not 410.
    var softTerminal =
      cLow0.indexOf("cycle_complete") >= 0 ||
      cLow0.indexOf("identity_complete") >= 0 ||
      cLow0.indexOf("identity_final") >= 0 ||
      cLow0 === "cool" ||
      cLow0 === "probe_complete" ||
      cLow0 === "analysis_terminal" ||
      cLow0 === "incomplete_ttl" ||
      cLow0 === "cycle_purged" ||
      cLow0 === "session_expired";
    haltState = {
      reason: reason || "halt",
      session_id: sid,
      code: c,
      at_ms: Date.now(),
      business_state: phaseForCode(c, reason),
      status: softTerminal ? 200 : 410,
      soft_close: !!softTerminal,
    };
    try {
      global.__GR_STOP_PROBE__ = true;
      global.__GR_SKIP_IDENTITY__ = true;
      global.__GR_HALT_UPLOADS__ = true;
      global.__GR_PHASE__ = phaseForCode(c, reason);
      global.__GR_UPLOAD_HALT__ = haltState;
      global.__GR_CYCLE_CLOSED__ = {
        session_id: sid,
        code: c,
        reason: reason,
        at_ms: haltState.at_ms,
        soft_close: !!softTerminal,
      };
      // Persist cool ONLY on true schedule-final / identity_complete / cycle_complete cool.
      // Cool is version-scoped (product_version stamp); version change invalidates via storage.
      // Do NOT stamp 24h cool on generic superseded/pagehide — that froze incomplete VT gap-fill.
      try {
        var S = global.GRStorage;
        var cLow = String(c || reason || "").toLowerCase();
        var coolOkHalt =
          cLow.indexOf("identity_complete") >= 0 ||
          cLow.indexOf("identity_final") >= 0 ||
          cLow.indexOf("schedule_final") >= 0 ||
          cLow.indexOf("cycle_complete") >= 0 ||
          cLow === "cool" ||
          cLow === "probe_complete" ||
          cLow === "analysis_terminal" ||
          !!global.__GR_BRAIN_SCHEDULE_FINAL__ ||
          !!global.__GR_HARD_FINAL__;
        // pagehide / superseded without schedule final → no cool stamp
        if (
          S &&
          S.setCoolUntil &&
          coolOkHalt &&
          !isPageUnloading()
        ) {
          var until = S.getCoolUntil && S.getCoolUntil();
          var now = Date.now();
          var ver =
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            (global.__GR_BOOT__ && (global.__GR_BOOT__.version || global.__GR_BOOT__.product_version)) ||
            global.__GR_PRODUCT_VERSION__ ||
            (haltState && haltState.product_version) ||
            undefined;
          if (!until || until <= now) {
            S.setCoolUntil(now + 24 * 60 * 60 * 1000, ver);
          } else {
            // Refresh version stamp on existing cool window (keeps cool tied to current version).
            S.setCoolUntil(until, ver);
          }
        }
      } catch (eCool) {}
    } catch (eG) {}
    // Drop pending except B10x EDH must-land (and explicit force_after_halt).
    if (pending.length) {
      var keepB10x = [];
      for (var pi = 0; pi < pending.length; pi++) {
        if (allowDespiteHalt(pending[pi])) keepB10x.push(pending[pi]);
        else haltedDrops++;
      }
      pending = keepB10x;
    }
    if (!pending.length) {
      cfg.concurrency = 0;
    } else {
      // Keep a small pump capacity so silicon B10x can finish after identity halt.
      if (cfg.concurrency < 2) cfg.concurrency = 2;
      try {
        setTimeout(function () {
          pump();
        }, 0);
      } catch (eP) {}
    }
    // Abort in-flight fetch so concurrent POSTs do not all land as 410 in Network.
    var ctrls = inflightCtrls.slice();
    inflightCtrls = [];
    for (var ai = 0; ai < ctrls.length; ai++) {
      try {
        if (ctrls[ai] && ctrls[ai].abort) ctrls[ai].abort();
      } catch (eAb) {}
    }
    try {
      global.dispatchEvent(
        new CustomEvent("gr-cycle-closed", {
          detail: {
            reason: haltState.reason,
            code: haltState.code,
            session_id: haltState.session_id,
          },
        })
      );
    } catch (eEv) {}
  }

  function isHaltedFor(item) {
    try {
      if (global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) return true;
    } catch (eH) {}
    if (!haltState) return false;
    var sid = sessionOf(item);
    if (!haltState.session_id) return true;
    if (!sid) return true;
    return sid === haltState.session_id;
  }

  /** EDH silicon deepen must still upload after identity halt / cool. */
  function isB10xBatch(batchId) {
    return String(batchId || "").indexOf("B10x_") === 0;
  }

  /** iss/54–57 secondary silicon/infra — must land ok or honest skip even after identity halt. */
  function isSecondaryInfraBatch(batchId) {
    var id = String(batchId || "");
    return (
      id === "B47_sab_clock" ||
      id === "B18_webgpu" ||
      id === "B46_audio_deep" ||
      id === "B10x_silicon_deep"
    );
  }

  /**
   * Re-open cycle to remint seal_grant (open lag / wiped grant).
   * Module-level so enqueue + postOne share one coalesced refresh.
   */
  function refreshSealGrant(sid) {
    if (global.__GR_SEAL_REFRESH_P__) return global.__GR_SEAL_REFRESH_P__;
    var base = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "");
    var cycle = String(
      sid ||
        global.__GR_SESSION_ID__ ||
        global.__GR_CYCLE_ID__ ||
        cfg.session_id ||
        ""
    );
    var vt = "";
    try {
      vt =
        global.__GR_VTID__ ||
        (global.GRStorage && GRStorage.visitorTerminalId && GRStorage.visitorTerminalId()) ||
        "";
    } catch (eV) {}
    var site =
      global.__GR_SITE_ID__ ||
      (global.__GR_BOOT__ && (global.__GR_BOOT__.site_id || global.__GR_BOOT__.siteId)) ||
      "";
    // embed gate: open 需带站点 embed_token。优先 __GR_BOOT__.embed_token，
    // 否则从 pin_url / 含 grt= 的 script 标签取(与 gr.boot embedTokenFromUrl 同源)。
    var etok = "";
    try {
      var bc = global.__GR_BOOT__ || {};
      etok = String(bc.embed_token || "");
      if (!etok) {
        var srcs = [];
        if (bc.pin_url) srcs.push(String(bc.pin_url));
        try {
          if (typeof document !== "undefined" && document.querySelectorAll) {
            var tags = document.querySelectorAll('script[src*="grt="]');
            for (var i = 0; i < tags.length; i++) srcs.push(String(tags[i].src || ""));
          }
        } catch (eQs0) {}
        for (var j = 0; j < srcs.length; j++) {
          var m = srcs[j].match(/[?&]grt=([^&]+)/);
          if (m) {
            etok = decodeURIComponent(m[1]);
            break;
          }
        }
      }
    } catch (eEt) {}
    var openUrl = base + "/v1/session/open";
    var openSame = false;
    try {
      openSame =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(openUrl, location.href).origin === location.origin;
    } catch (eOpenSo) {
      openSame = String(openUrl).charAt(0) === "/";
    }
    global.__GR_SEAL_REFRESH_P__ = fetch(openUrl, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        cycle_id: cycle || undefined,
        session_id: cycle || undefined,
        visitor_terminal_id: vt || undefined,
        site_id: site || undefined,
        embed_token: etok || undefined,
        meta: { fe: "seal_refresh", inject_path: cfg.inject_path || "app" },
      }),
      // CF orange: same-origin must send cf_clearance; cross-origin include when CORS allows
      credentials: openSame ? "same-origin" : "include",
      mode: openSame ? "same-origin" : "cors",
      cache: "no-store",
    })
      .then(function (r) {
        return r.json().catch(function () {
          return {};
        });
      })
      .then(function (j) {
        if (j && j.seal_grant) {
          try {
            global.__GR_SEAL_GRANT__ = j.seal_grant;
            global.__GR_BOOT__ = global.__GR_BOOT__ || {};
            global.__GR_BOOT__.seal_grant = j.seal_grant;
            if (j.require_sealed_ingest) {
              global.__GR_REQUIRE_SEALED__ = true;
              global.__GR_BOOT__.require_sealed_ingest = true;
            }
            if (global.GRSeal) {
              if (GRSeal.setRequireSealed && j.require_sealed_ingest) GRSeal.setRequireSealed(true);
              if (GRSeal.setGrant) GRSeal.setGrant(j.seal_grant);
              if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            }
            var ns = j.cycle_id || j.session_id || "";
            if (ns) {
              global.__GR_SESSION_ID__ = ns;
              global.__GR_CYCLE_ID__ = ns;
              cfg.session_id = ns;
            }
          } catch (eG) {}
          return true;
        }
        return false;
      })
      .catch(function () {
        return false;
      })
      .then(function (ok) {
        global.__GR_SEAL_REFRESH_P__ = null;
        try {
          if (ok) pump();
        } catch (eK) {}
        return ok;
      });
    return global.__GR_SEAL_REFRESH_P__;
  }

  function allowDespiteHalt(item) {
    if (!item) return false;
    if (item.force_after_halt || item.allow_during_stop) return true;
    if (isB10xBatch(item.batch_id)) return true;
    if (isSecondaryInfraBatch(item.batch_id)) return true;
    return false;
  }

  function uploadsBlocked(item) {
    if (allowDespiteHalt(item)) return false;
    if (haltState || (function () {
      try {
        return !!(global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__);
      } catch (e) {
        return false;
      }
    })()) {
      return true;
    }
    return isHaltedFor(item || {});
  }

  /** True when uploads go same-origin first-party (/g5) — lower handshake cost, no CORS preflight. */
  function isFirstPartyMode() {
    try {
      if (global.__GR_FIRST_PARTY__) return true;
      var b = global.__GR_BOOT__ || {};
      if (b.first_party || b.firstParty) return true;
      var raw = String(cfg.apiBase || b.apiBase || "").trim();
      if (raw.charAt(0) === "/") return true;
      if (raw && typeof location !== "undefined" && location.origin) {
        return new URL(raw, location.href).origin === location.origin;
      }
    } catch (eFp) {}
    return false;
  }

  /** Resolve relative first-party apiBase (/g5) against page origin. */
  function resolvedApiBase() {
    var raw = String(cfg.apiBase || global.__GR_BOOT__ && global.__GR_BOOT__.apiBase || "").trim();
    if (!raw && (global.__GR_FIRST_PARTY__ || (global.__GR_BOOT__ && global.__GR_BOOT__.first_party))) {
      raw = "/g5";
    }
    if (!raw) return "";
    if (/^https?:\/\//i.test(raw)) return raw.replace(/\/$/, "");
    if (raw.indexOf("//") === 0) {
      try {
        return (location.protocol + raw).replace(/\/$/, "");
      } catch (e0) {
        return ("https:" + raw).replace(/\/$/, "");
      }
    }
    if (raw.charAt(0) !== "/") raw = "/" + raw;
    raw = raw.replace(/\/$/, "");
    try {
      return (location.origin + raw).replace(/\/$/, "");
    } catch (e1) {
      return raw;
    }
  }

  /**
   * iss/70 P2: first-party may raise *caps*, never force high floor via max-stack.
   * effectiveUploadCap() is the final gate.
   */
  function applyFirstPartyPerfHints() {
    if (!isFirstPartyMode()) return;
    try {
      var mc = Number(global.__GR_UPLOAD_CONCURRENCY__ || 0);
      var mr = Number(global.__GR_MID_RAMP_CONCURRENCY__ || 0);
      var man =
        global.__GR_MANIFEST__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.manifest) ||
        null;
      var bh = (man && man.brain_hints) || {};
      var maxC = Number(cfg.concurrency_max) || 6;
      // Requests may raise concurrency_max, not stamp concurrency to 12/16.
      if (mc > 0) cfg.concurrency_max = Math.min(8, Math.max(maxC, Math.min(mc, 8)));
      if (mr > 0) cfg.mid_ramp_concurrency = Math.min(cfg.concurrency_max, Math.max(cfg.mid_ramp_concurrency || 0, Math.min(mr, 6)));
      if (bh.upload_concurrency != null) {
        var req = Number(bh.upload_concurrency) || 0;
        if (req > 0) cfg.concurrency_max = Math.min(8, Math.max(cfg.concurrency_max || 6, Math.min(req, 8)));
      }
      if (bh.mid_ramp_concurrency != null) {
        var reqM = Number(bh.mid_ramp_concurrency) || 0;
        if (reqM > 0) {
          cfg.mid_ramp_concurrency = Math.min(
            cfg.concurrency_max || 6,
            Math.max(cfg.mid_ramp_concurrency || 0, Math.min(reqM, 6))
          );
        }
      }
      // Keep base concurrency at floor..max, never inflate to 12+.
      cfg.concurrency = Math.min(
        cfg.concurrency_max || 6,
        Math.max(cfg.concurrency_floor || 1, Number(cfg.concurrency) || 3)
      );
    } catch (eMan) {}
  }

  function postOne(item) {
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var base = resolvedApiBase() || String(cfg.apiBase || "").replace(/\/$/, "");
    var url = base + "/v1/ingest";
    var productVer =
      (global.__GR_PRODUCT_VERSION__ ||
        (global.__GR_BOOT__ &&
          (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
        "") + "";
    // iss/72: freeze once at capture boundary; never mutate material after freeze.
    freezeCapture(item);
    var payload = item.payload || {};
    // Integrity: frozen wire material must match capture; restore if collector aliased.
    try {
      if (item._capture_frozen && item._frozen_payload_json) {
        var nowS = stableStringify((payload && payload.fields) || {});
        if (nowS !== item._frozen_payload_json) {
          var restored = false;
          try {
            var snap =
              item._frozen_fields && typeof item._frozen_fields === "object"
                ? deepCloneJson(item._frozen_fields)
                : null;
            if (snap && typeof snap === "object") {
              if (!item.payload || typeof item.payload !== "object") item.payload = {};
              item.payload.fields = snap;
              payload = item.payload;
              item.payload_hash = payloadHashOf(payload);
              item.material_hash = item.payload_hash;
              restored = stableStringify(snap) === item._frozen_payload_json;
            }
          } catch (eRest) {
            restored = false;
          }
          try {
            if (global.GROps && GROps.report) {
              // Restored shared-ref churn → info-level noise control; true unrestorable → warn.
              GROps.report(
                restored ? "capture_mutation_restored" : "capture_mutated_blocked",
                "upload",
                { batch_id: item.batch_id, restored: !!restored },
                restored ? "info" : "warn"
              );
            }
          } catch (eM) {}
        }
      }
    } catch (ePv) {}
    var planEpoch = null;
    try {
      if (item.plan_epoch != null) planEpoch = Number(item.plan_epoch);
      else if (global.__GR_PLAN_EPOCH__ != null) planEpoch = Number(global.__GR_PLAN_EPOCH__);
      else if (global.GRSessionScheduler && GRSessionScheduler.getPlanEpoch) {
        planEpoch = Number(GRSessionScheduler.getPlanEpoch());
      }
      if (!isFinite(planEpoch) || planEpoch <= 0) planEpoch = null;
    } catch (ePe) {
      planEpoch = null;
    }
    var body = {
      session_id: item.session_id || cfg.session_id || global.__GR_SESSION_ID__ || "",
      batch_id: item.batch_id,
      source: item.source || "main",
      analyze: false,
      inject_path: item.inject_path || cfg.inject_path,
      product_version: productVer || undefined,
      payload: payload,
      capture_id: item.capture_id || undefined,
      material_generation: item.material_generation != null ? item.material_generation : undefined,
      payload_hash: item.payload_hash || undefined,
      material_hash: item.material_hash || item.payload_hash || undefined,
      // iss/72: stamp plan_epoch so server can reject stale route material.
      plan_epoch: planEpoch != null ? planEpoch : undefined,
      // attempt_id is transport-only — changes every send, not material identity.
      attempt_id:
        "att_" +
        String(item._transport_attempts || 0) +
        "_" +
        String(Date.now()),
      source_kind: "fe",
      realm_kind: (function () {
        try {
          var src = String(item.source || "main").toLowerCase();
          if (src.indexOf("worker") >= 0) return "worker";
          if (src.indexOf("sandbox") >= 0) return "sandbox_iframe";
          if (src.indexOf("iframe") >= 0) return "iframe";
          return "document";
        } catch (eRk) {
          return "document";
        }
      })(),
      probe_method_id:
        (item.probe_method_id ||
          item.method_id ||
          (item.method && item.method.method_id) ||
          undefined) || undefined,
    };
    // Never beacon after halt — pagehide would re-flood completed cycles.
    if (uploadsBlocked(item)) {
      return Promise.reject(
        Object.assign(new Error("upload_halted"), { terminal: true, status: 410, silent: true })
      );
    }
    var sameOrigin = false;
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    var ctrl = null;
    try {
      if (typeof AbortController !== "undefined") {
        ctrl = new AbortController();
        try {
          ctrl.__gr_batch_id = String((item && item.batch_id) || "");
        } catch (eTag) {}
        inflightCtrls.push(ctrl);
      }
    } catch (eC) {}
    var fp = sameOrigin || isFirstPartyMode();
    // Gecko/Edge: hard anchors must prefer first-party same-origin (CORS dual-fire 5xx).
    var hard = isHardAnchorBatch(item.batch_id);
    var engineGecko = false;
    var engineEdge = false;
    try {
      var uaE = String((global.navigator && navigator.userAgent) || "");
      // No InstallTrigger (deprecated — typeof alone warns in Firefox).
      try {
        engineGecko =
          typeof global.mozInnerScreenX === "number" ||
          /Firefox\//.test(uaE) ||
          /FxiOS\//.test(uaE) ||
          (/Gecko\//.test(uaE) && !/like Gecko/.test(uaE));
      } catch (eG) {
        engineGecko = /Firefox\//.test(uaE);
      }
      engineEdge = /Edg\//.test(uaE);
    } catch (eEg) {}
    // Gecko/Edge: always first-party same-origin ingest (CORS dual-fire → NetworkError / mid incomplete).
    var forceFpEngine = engineGecko || engineEdge;
    if (forceFpEngine && !sameOrigin) {
      try {
        var p = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
        url = p + "/v1/ingest";
        sameOrigin = true;
        fp = true;
      } catch (eFp) {}
    }

    // Session seal: async prepare (grant from open). Beacon cannot await crypto — skip when sealing.
    // CRITICAL under REQUIRE_SEALED: never plain-fallback (server 426 storm). Lab-only plain when not required.
    function sealNeedNow() {
      try {
        if (global.__GR_FORCE_PLAIN_INGEST__) return false;
        if (global.__GR_SEEN_SEALED_REQUIRED__ || global.__GR_REQUIRE_SEALED__) return true;
        if (global.GRSeal && typeof GRSeal.isRequireSealed === "function" && GRSeal.isRequireSealed())
          return true;
        var b = global.__GR_BOOT__ || {};
        if (b.require_sealed_ingest || (b.policy && b.policy.require_sealed_ingest)) return true;
        // Prod first-party: assume sealed before open/bootstrap lands (kill plain→426 race).
        if (
          (global.__GR_FIRST_PARTY__ || b.first_party || b.firstParty) &&
          (b.inject_path === "nginx" ||
            b.injectPath === "nginx" ||
            (b.env_id && String(b.env_id).indexOf("prod") === 0))
        ) {
          return true;
        }
      } catch (eN) {}
      return false;
    }
    /** Wait for open.seal_grant (or timeout) when sealed required — no plain attempt. */
    function waitOpenSealGrant(timeoutMs) {
      timeoutMs = timeoutMs == null ? 16000 : timeoutMs;
      try {
        if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
          return Promise.resolve(true);
        }
        if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
          if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
          return Promise.resolve(true);
        }
      } catch (e0) {}
      if (global.__GR_SEAL_READY_P__) {
        return Promise.race([
          global.__GR_SEAL_READY_P__.then(function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            return !!(
              (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) ||
              (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64)
            );
          }),
          new Promise(function (resolve) {
            setTimeout(function () {
              resolve(false);
            }, timeoutMs);
          }),
        ]);
      }
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          try {
            if (global.GRSeal && GRSeal.grantRawValid && GRSeal.grantRawValid()) {
              return resolve(true);
            }
            if (global.__GR_SEAL_GRANT__ && global.__GR_SEAL_GRANT__.key_b64) {
              if (global.GRSeal && GRSeal.setGrant) GRSeal.setGrant(global.__GR_SEAL_GRANT__);
              return resolve(true);
            }
          } catch (e1) {}
          if (Date.now() - start > timeoutMs) return resolve(false);
          setTimeout(tick, 35);
        }
        tick();
      });
    }

    function ensureSealModule() {
      if (global.GRSeal && typeof GRSeal.prepareUpload === "function") {
        return Promise.resolve(true);
      }
      if (!sealNeedNow()) return Promise.resolve(false);
      if (global.__GR_SEAL_LOAD_P__) return global.__GR_SEAL_LOAD_P__;
      global.__GR_SEAL_LOAD_P__ = new Promise(function (resolve) {
        try {
          var basePath = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5").replace(/\/$/, "") || "/g5";
          var ver =
            (global.__GR_SERVER_PRODUCT_VERSION__ ||
              global.__GR_PRODUCT_VERSION__ ||
              (global.__GR_BOOT__ && (global.__GR_BOOT__.product_version || global.__GR_BOOT__.version)) ||
              "") + "";
          var src =
            basePath +
            "/dist/gr.seal.min.js" +
            (ver ? "?v=" + encodeURIComponent(ver) : "");
          var s = document.createElement("script");
          s.async = true;
          s.src = src;
          s.onload = function () {
            try {
              if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
            } catch (eA) {}
            resolve(!!(global.GRSeal && GRSeal.prepareUpload));
          };
          s.onerror = function () {
            resolve(false);
          };
          (document.head || document.documentElement).appendChild(s);
        } catch (eL) {
          resolve(false);
        }
      });
      return global.__GR_SEAL_LOAD_P__;
    }
    // Seal circuit: stop seal storms without re-collect thrash (GPI Phase C).
  function sealCircuitOpen() {
    try {
      if (global.__GR_SEAL_WASM_DEAD__) return true;
      var c = global.__GR_SEAL_CIRCUIT__;
      if (c && c.open && c.until && Date.now() < c.until) return true;
      if (c && c.open && c.until && Date.now() >= c.until) {
        global.__GR_SEAL_CIRCUIT__ = { open: false, fails: 0, until: 0 };
      }
    } catch (e) {}
    return false;
  }
  function sealCircuitTrip(why) {
    try {
      var c = global.__GR_SEAL_CIRCUIT__ || { fails: 0 };
      c.fails = (c.fails || 0) + 1;
      if (c.fails >= 3) {
        c.open = true;
        c.until = Date.now() + 30000;
        c.why = String(why || "seal_fail");
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_circuit_open",
              "upload",
              { fails: c.fails, until: c.until, why: c.why },
              "warn"
            );
          }
        } catch (eO) {}
      }
      global.__GR_SEAL_CIRCUIT__ = c;
    } catch (e2) {}
  }

  var sealPrep = ensureSealModule()
      .then(function () {
        if (sealCircuitOpen()) {
          var ec = new Error("seal_circuit_open");
          ec.code = "seal_circuit_open";
          ec.seal_failed = true;
          ec.seal_circuit = true;
          throw ec;
        }
        // When sealed required: never race plain — wait open grant first (hard batches especially).
        if (!sealNeedNow()) return true;
        var waitMs = hard || isDeepenBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 20000 : 14000;
        return waitOpenSealGrant(waitMs).then(function (ok) {
          if (ok) return true;
          // Open lag / wiped grant: remint via session open once, then short wait.
          return refreshSealGrant(sessionOf(item)).then(function (got) {
            if (!got) return false;
            return waitOpenSealGrant(4000);
          });
        });
      })
      .then(function () {
      if (!(global.GRSeal && typeof GRSeal.prepareUpload === "function")) {
        if (sealNeedNow()) {
          var eMiss = new Error("seal_module_missing");
          eMiss.code = "seal_failed";
          eMiss.seal_failed = true;
          throw eMiss;
        }
        return {
          url: url,
          body: JSON.stringify(body),
          sealed: false,
        };
      }
      try {
        if (GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eAd0) {}
      // Prefer authoritative session id after open supersede.
      try {
        var authSid =
          (global.__GR_SESSION_ID__ || cfg.session_id || body.session_id || "") + "";
        if (authSid && body.session_id && authSid !== body.session_id) {
          // Only rewrite when auth looks like a real cycle and grant matches it.
          if (
            GRSeal.grantValid &&
            GRSeal.grantValid(authSid) &&
            String(authSid).indexOf("cycle_") === 0
          ) {
            body.session_id = authSid;
            item.session_id = authSid;
          }
        }
      } catch (eSid) {}
      return GRSeal.prepareUpload(base, body).catch(function (eSeal) {
        var need = sealNeedNow();
        if (need) {
          var es = eSeal || new Error("seal_failed");
          es.code = es.code || "seal_failed";
          es.seal_failed = true;
          // Permanent WASM death (unsupported / not binary / load fail): stop seal retry storm.
          var msgSeal = String(es.message || es || "");
          var wasmDead =
            !!global.__GR_SEAL_WASM_DEAD__ ||
            /seal_wasm_unsupported|wasm_not_binary|wasm_http_|seal_wasm_required/i.test(msgSeal);
          if (wasmDead) {
            try {
              global.__GR_SEAL_WASM_DEAD__ = 1;
              es.code = /unsupported/i.test(msgSeal) ? "seal_wasm_unsupported" : "seal_failed";
              es.terminal = false; // still allow remint if grant path heals; budget handled below
              es.seal_wasm_dead = true;
            } catch (eDead) {}
          }
          try {
            // Cap ops: crawlers emitted 60–70× upload_5xx/session on seal_wasm_required.
            global.__GR_SEAL_OPS_N__ = (global.__GR_SEAL_OPS_N__ || 0) + 1;
            if (global.__GR_SEAL_OPS_N__ <= 3 && global.GROps && GROps.uploadHttp) {
              GROps.uploadHttp(0, item && item.batch_id, {
                seal_failed: true,
                seal_wasm_dead: !!wasmDead,
                code: es.code,
                err: msgSeal.slice(0, 160),
                final: !!wasmDead && global.__GR_SEAL_OPS_N__ >= 2,
              });
            }
          } catch (eOpsS) {}
          throw es;
        }
        try {
          if (global.GROps && GROps.report) {
            GROps.report(
              "seal_fallback_plain",
              "upload",
              {
                batch: String((item && item.batch_id) || "").slice(0, 48),
                err: String((eSeal && eSeal.message) || eSeal || "").slice(0, 80),
              },
              "warn"
            );
          }
        } catch (eRep) {}
        return {
          url: base + "/v1/ingest",
          body: JSON.stringify(body),
          sealed: false,
        };
      });
    });

    return sealPrep.then(function (wire) {
    url = wire.url || url;
    var bodyStr = wire.body || JSON.stringify(body);
    var sealed = !!wire.sealed;
    // Keep item session aligned with sealed envelope (supersede path).
    try {
      if (wire.session_id && item) item.session_id = wire.session_id;
    } catch (eWs) {}
    try {
      sameOrigin =
        typeof location !== "undefined" &&
        location.origin &&
        new URL(url, location.href).origin === location.origin;
    } catch (eSo2) {
      sameOrigin = String(url || "").charAt(0) === "/";
    }
    fp = sameOrigin || isFirstPartyMode();

    // Sealed path: never sendBeacon (async crypto; envelope size). Use fetch+keepalive on pagehide.
    if (hiding && !sealed && global.navigator && navigator.sendBeacon) {
      try {
        var blob = new Blob([bodyStr], { type: "text/plain;charset=UTF-8" });
        if (navigator.sendBeacon(url, blob)) return Promise.resolve({ ok: true, via: "beacon" });
      } catch (e) {}
    }
    // Gecko: avoid keepalive on large hard plain bodies (broken-pipe 5xx).
    // Sealed+hide: always keepalive so unload does not drop encrypted batch (no sendBeacon).
    var useKeepalive = false;
    if (sealed && hiding) {
      useKeepalive = true;
    } else if (hiding || (fp && hard && !engineGecko && !engineEdge)) {
      useKeepalive = true;
    }
    var init = {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: bodyStr,
      // Same-origin: send CF cookies; cross-origin: include when CORS credentials allowed
      credentials: sameOrigin ? "same-origin" : "include",
      mode: sameOrigin ? "same-origin" : "cors",
      keepalive: !!useKeepalive,
    };
    if (ctrl) init.signal = ctrl.signal;
    // Abort hard uploads that hang so SLA can retry.
    // Gecko: hard multipath payloads are large — short abort caused timeout storms
    // (upload_timeout_abort → probe_fail_budget → mid blocked).
    var abortTo = null;
    if (ctrl && (hard || isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id))) {
      try {
        // Heavy deepen needs longer: large sealed body + compress.
        var hardAbortMs = isHeavyBatch(item.batch_id) || isDeepenBatch(item.batch_id)
          ? engineGecko || engineEdge
            ? 45000
            : 36000
          : engineGecko || engineEdge
            ? 28000
            : 22000;
        abortTo = setTimeout(function () {
          try {
            if (ctrl) ctrl.__gr_timeout_abort = 1;
            ctrl.abort();
          } catch (eAb) {}
        }, hardAbortMs);
      } catch (eT) {}
    }
    // First / high-priority batches: high fetch priority (v57 p50 path)
    try {
      if (sent === 0 || (item.priority || 0) >= 90 || hard) {
        init.priority = "high";
      } else if (fp) {
        // Remaining packs: still elevated so relay hop is not starved by page assets
        init.priority = "high";
      }
    } catch (eP) {}
    return fetch(url, init)
      .finally(function () {
        if (abortTo) {
          try {
            clearTimeout(abortTo);
          } catch (eC2) {}
        }
      })
      .then(function (r) {
        // Drop controller from list when response arrives
        if (ctrl) {
          var ix = inflightCtrls.indexOf(ctrl);
          if (ix >= 0) inflightCtrls.splice(ix, 1);
        }
        // If we halted while request was in flight, do not parse/retry as error flood.
        if (uploadsBlocked(item) && r.status === 410) {
          applyHalt("session_expired", sessionOf(item), "cycle_complete");
          var errH = new Error("upload_halted");
          errH.terminal = true;
          errH.status = 410;
          errH.silent = true;
          errH.code = "cycle_complete";
          throw errH;
        }
        return r
          .json()
          .catch(function () {
            // HTTP 2xx with empty/non-JSON body is still a successful transport —
            // do NOT invent ok:false (was flooding upload_biz_reject with "HTTP 200").
            if (r.ok) {
              return { ok: true, empty_body: true, http: r.status };
            }
            return { ok: false, error: "HTTP " + r.status };
          })
          .then(function (j) {
            var term = parseTerminal(r.status, j || {});
            if (term.terminal) {
              // Halt BEFORE throwing so sibling inflight sees blocked flag.
              applyHalt(term.code || "session_expired", sessionOf(item), term.code);
              // Soft 200 close: resolve quietly (no throw → no red XHR / NS_BINDING_ABORTED cascade).
              if (term.soft_close || (r.status === 200 && term.terminal)) {
                try {
                  if (global.GROps && GROps.report) {
                    GROps.report(
                      "upload_soft_cycle_closed",
                      "upload",
                      {
                        code: term.code,
                        batch: String((item && item.batch_id) || "").slice(0, 48),
                      },
                      "info"
                    );
                  }
                } catch (eSoft) {}
                return j || { ok: true, accepted: false, halt_uploads: true };
              }
              var err = new Error((j && j.error) || "HTTP " + r.status);
              err.terminal = true;
              err.status = r.status;
              err.code = term.code;
              err.body = j;
              err.silent = r.status === 410 || r.status === 200;
              throw err;
            }
            // Transport OK: never treat as biz reject (even if body omitted ok).
            if (r.ok) {
              if (j && j.ok === false && j.error) {
                // Explicit server business reject on 200 — report warn, still accept storage if ingest landed.
                try {
                  if (global.GROps && GROps.uploadHttp) {
                    GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                      ok: false,
                      err: j.error,
                      explicit_biz: true,
                    });
                  }
                } catch (eOpsU0) {}
              }
              // Prior network fail for this batch → recovery event (vtid-tagged).
              try {
                var bidOk = String((item && (item.batch_id || item.pack_id)) || "");
                global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
                var prevFails = global.__GR_NET_FAIL_N__[bidOk] || 0;
                if (prevFails > 0 && global.GROps && GROps.uploadRecovered) {
                  GROps.uploadRecovered(bidOk, {
                    prior_network_fails: prevFails,
                    http: r.status,
                    sealed: !!sealed,
                  });
                  delete global.__GR_NET_FAIL_N__[bidOk];
                }
              } catch (eRec) {}
              return j || { ok: true };
            }
            // HTTP 403/503: sniff CF "I'm Under Attack" / challenge HTML.
            if (r.status === 403 || r.status === 503 || r.status === 429) {
              try {
                var snip =
                  (j && (j.error || j.message || JSON.stringify(j).slice(0, 120))) ||
                  "HTTP " + r.status;
                var snipL = String(snip).toLowerCase();
                var cfHit =
                  /challenge|cf-mitigated|just a moment|attention required|under attack|cloudflare|cf-ray|access denied/i.test(
                    snipL
                  );
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(r.status, item && (item.batch_id || item.pack_id), {
                    ok: false,
                    err: String(snip).slice(0, 100),
                    body_snip: String(snip).slice(0, 100),
                    net_class: cfHit ? "edge_challenge" : "http_" + r.status,
                    cf_challenge: !!cfHit,
                    will_retry: true,
                  });
                }
                var eCf = new Error(String(snip).slice(0, 120));
                eCf.status = r.status;
                eCf.body = j;
                eCf.network = !!cfHit; // retriable when edge challenge
                eCf.code = cfHit ? "upload_edge_challenge" : "upload_http_" + r.status;
                eCf.cf_challenge = !!cfHit;
                throw eCf;
              } catch (eCfThrow) {
                if (eCfThrow && eCfThrow.status) throw eCfThrow;
              }
            }
            if (j && j.ok === false) {
              try {
                if (global.GROps && GROps.uploadHttp) {
                  GROps.uploadHttp(
                    r.status,
                    item && (item.batch_id || item.pack_id),
                    { ok: j && j.ok, err: j && j.error }
                  );
                }
              } catch (eOpsU) {}
              var e2 = new Error((j && j.error) || "HTTP " + r.status);
              e2.status = r.status;
              e2.body = j;
              // Broken pipe / client abort often surfaces as TypeError network fail upstream.
              e2.network = false;
              // 426 sealed_ingest_required: flip require flag, never retry plain.
              try {
                var errTxt = String((j && j.error) || "");
                if (
                  r.status === 426 ||
                  /sealed_ingest_required/i.test(errTxt)
                ) {
                  global.__GR_SEEN_SEALED_REQUIRED__ = true;
                  global.__GR_REQUIRE_SEALED__ = true;
                  global.__GR_BOOT__ = global.__GR_BOOT__ || {};
                  global.__GR_BOOT__.require_sealed_ingest = true;
                  if (global.GRSeal && GRSeal.setRequireSealed) {
                    GRSeal.setRequireSealed(true);
                  }
                  e2.code = "sealed_required";
                  e2.seal_required = true;
                  e2.seal_failed = true; // retriable path (clear sentKeys for hard)
                }
              } catch (e426) {}
              throw e2;
            }
            // Non-JSON error body with 426
            if (r.status === 426) {
              try {
                global.__GR_SEEN_SEALED_REQUIRED__ = true;
                global.__GR_REQUIRE_SEALED__ = true;
                if (global.GRSeal && GRSeal.setRequireSealed) GRSeal.setRequireSealed(true);
              } catch (e426b) {}
              var e426e = new Error("sealed_ingest_required");
              e426e.status = 426;
              e426e.code = "sealed_required";
              e426e.seal_required = true;
              e426e.seal_failed = true;
              throw e426e;
            }
            return j;
          });
      })
      .catch(function (e) {
        if (ctrl) {
          var iy = inflightCtrls.indexOf(ctrl);
          if (iy >= 0) inflightCtrls.splice(iy, 1);
        }
        // Aborted due to halt / true unload / timeout — silent (never flood ops as network).
        // Tab background (PAGE_BACKGROUNDED) is NOT terminal — keep retrying for maximize probe.
        var msgE = String((e && (e.message || e.name)) || e || "");
        var unloading = isPageUnloading();
        if (
          e &&
          (e.name === "AbortError" ||
            /abort/i.test(msgE) ||
            (haltState && !e.status) ||
            unloading ||
            global.__GR_HALT_UPLOADS__)
        ) {
          var ea = new Error("upload_halted");
          ea.terminal = !!haltState || unloading;
          ea.status = haltState ? 410 : 0;
          ea.silent = true;
          ea.code = (haltState && haltState.code) || (unloading ? "pagehide" : "halt");
          // Soft timeout abort (hard hang): allow retry, not terminal cycle close.
          if (
            !haltState &&
            !unloading &&
            (/abort/i.test(msgE) || (ctrl && ctrl.__gr_timeout_abort))
          ) {
            ea.terminal = false;
            ea.silent = true;
            ea.network = false; // do not flood upload_network
            ea.status = 0;
            ea.code = "upload_timeout_abort";
            ea.timeout_abort = true;
          }
          throw ea;
        }
        // Network / broken-pipe: report once per batch (not every retry).
        // Classify CF challenge / abort / background so ops is not pure "error flood".
        if (e && !e.status) {
          try {
            var bidN = String((item && (item.batch_id || item.pack_id)) || "");
            var hideN = unloading;
            var netKey = "net:" + bidN;
            global.__GR_NET_REPORT__ = global.__GR_NET_REPORT__ || Object.create(null);
            global.__GR_NET_FAIL_N__ = global.__GR_NET_FAIL_N__ || Object.create(null);
            var lastN = global.__GR_NET_REPORT__[netKey] || 0;
            var nowN = Date.now();
            var msgLow = String(msgE || "").toLowerCase();
            var netClass = "fetch_fail";
            if (e.timeout_abort || /abort/i.test(msgLow)) netClass = "abort";
            else if (
              /challenge|cf-mitigated|just a moment|attention required|access denied|under attack|cf-ray|cloudflare/i.test(
                msgLow
              )
            )
              netClass = "edge_challenge";
            else if (isPageBackgrounded()) netClass = "backgrounded";
            // Do not count silent/timeout aborts as network flood.
            if (e.silent || e.timeout_abort || netClass === "abort") {
              e.network = false;
              e.status = 0;
              e.code = e.code || "upload_timeout_abort";
            } else {
              global.__GR_NET_FAIL_N__[bidN] = (global.__GR_NET_FAIL_N__[bidN] || 0) + 1;
              var attN = global.__GR_NET_FAIL_N__[bidN];
              var maxAtt = maxAttemptsForItem(item);
              var willRetryN = attN < maxAtt && !hideN;
              if (
                !hideN &&
                nowN - lastN > 12000 &&
                global.GROps &&
                GROps.uploadHttp
              ) {
                global.__GR_NET_REPORT__[netKey] = nowN;
                // Sub-classify http:0 Failed to fetch (not server HTTP status).
                // online=false → offline; else client_fetch_fail (conn limit / DNS / TLS / CORS).
                var online =
                  typeof navigator !== "undefined" && navigator.onLine != null
                    ? !!navigator.onLine
                    : null;
                if (online === false) netClass = "offline";
                else if (netClass === "fetch_fail") netClass = "client_fetch_fail";
                var apiBaseN = "";
                var apiHostN = "";
                try {
                  apiBaseN = String(
                    cfg.apiBase ||
                      (global.__GR_BOOT__ && global.__GR_BOOT__.apiBase) ||
                      global.__GR_API_BASE__ ||
                      "/g5"
                  );
                  apiHostN =
                    apiBaseN.indexOf("http") === 0
                      ? apiBaseN.split("/").slice(0, 3).join("/")
                      : location && location.host
                        ? String(location.protocol) + "//" + location.host + apiBaseN
                        : apiBaseN;
                } catch (eAb) {}
                // Remember last net_class per batch for exhaust reports.
                try {
                  global.__GR_NET_CLASS_LAST__ =
                    global.__GR_NET_CLASS_LAST__ || Object.create(null);
                  global.__GR_NET_CLASS_LAST__[bidN] = {
                    net_class: netClass,
                    transport: "no_http_response",
                    ts: nowN,
                  };
                } catch (eMem) {}
                GROps.uploadHttp(0, bidN, {
                  network: true,
                  err: msgE.slice(0, 80),
                  net_class: netClass,
                  // Not an HTTP response from API — browser never got status line.
                  transport: "no_http_response",
                  fail_class_hint:
                    netClass === "edge_challenge"
                      ? "edge_challenge"
                      : netClass === "offline"
                        ? "client_offline"
                        : netClass === "abort" || netClass === "backgrounded"
                          ? "client_abort"
                          : "network_env",
                  impact_band_hint: isHeavyBatch(bidN) ? "deepen_optional" : "ops_attention",
                  api_base: apiBaseN.slice(0, 80),
                  api_host: String(apiHostN).slice(0, 120),
                  online: online,
                  backgrounded: isPageBackgrounded(),
                  unloading: !!hideN || isPageUnloading(),
                  pagehide: !!hideN,
                  short_visit: !!hideN || isPageUnloading(),
                  commercial_hard: isCommercialHardForOps(bidN),
                  commercial_land_fast: isCommercialLandFast(bidN),
                  attempt: attN,
                  max_attempts: maxAtt,
                  will_retry: willRetryN,
                  final: !willRetryN,
                  heavy: isHeavyBatch(bidN),
                });
              }
              e.network = true;
              e.status = 0;
              e.code = "upload_network";
            }
          } catch (eNet) {}
        }
        throw e;
      });
    });
  }

  function maxAttempts() {
    return hiding ? cfg.max_attempts_hide : cfg.max_attempts_alive;
  }

  function isRpaBatch(batchId) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.isRpaBatch) {
        return !!GRProbeLifecycle.isRpaBatch(batchId);
      }
    } catch (eR) {}
    var id = String(batchId || "");
    return id === "B11_interaction" || id.indexOf("B11_") === 0;
  }

  function maxAttemptsForItem(item) {
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.maxAttemptsFor) {
        var cap = GRProbeLifecycle.maxAttemptsFor(item && item.batch_id);
        if (hiding) return Math.min(cap, cfg.max_attempts_hide || 4);
        // RPA continuous: hard cap 3 alive, 2 on hide — never storm soft_exhausted.
        if (isRpaBatch(item && item.batch_id)) {
          return Math.min(cap, hiding ? 2 : 3);
        }
        return cap;
      }
    } catch (eM) {}
    if (isRpaBatch(item && item.batch_id)) return hiding ? 2 : 3;
    return maxAttempts();
  }

  /**
   * B10 + Lane-C must-land: beat short-visit / pagehide window.
   * Must NOT inherit deepen/heavy 4s+ stretch (lifecycle marks these heavy).
   */
  function isCommercialLandFast(batchId) {
    var id = String(batchId || "");
    return (
      id === "B10_hw_curves" ||
      id === "mid.curves" ||
      id === "B10x_silicon_noderiv" ||
      id === "B10x_silicon_rint" ||
      id === "B10x_silicon_ulp"
    );
  }

  function backoffMsForItem(item, attempt) {
    var a = Math.max(0, (attempt || 1) - 1);
    // Commercial land path: front-load retries inside ~2–4s dwell + keepalive.
    if (isCommercialLandFast(item && item.batch_id)) {
      // 120, 360, 680, 1080, 1560… cap 2000
      return Math.min(2000, 120 + a * 240 + a * a * 40);
    }
    try {
      if (global.GRProbeLifecycle && GRProbeLifecycle.backoffMsFor) {
        return GRProbeLifecycle.backoffMsFor(item && item.batch_id, attempt);
      }
    } catch (eB) {}
    return Math.min(20000, 1000 * Math.pow(2, a));
  }

  function scheduleRetry(item, attempt) {
    var delay = backoffMsForItem(item, attempt);
    // Heavy network fails: longer backoff so browser connection budget recovers.
    // Never stretch commercial land — short visits lose B10/Lane-C otherwise.
    try {
      var bid = item && item.batch_id;
      if (!isCommercialLandFast(bid) && isHeavyBatch(bid)) {
        delay = Math.max(delay, 4000 + Math.min(20000, (attempt || 1) * 3000));
      }
    } catch (eHb) {}
    // iss/70: transport-only retry — keep frozen capture; do not re-collect.
    try {
      freezeCapture(item);
      transportRetryCount++;
      item._transport_attempts = (item._transport_attempts || 0) + 1;
    } catch (eTr) {}
    setTimeout(function () {
      if (haltState && !allowDespiteHalt(item)) return;
      if (uploadsBlocked(item)) return;
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          return;
        }
      } catch (eA) {}
      // Coalesce force retries (same key) — avoid pending storms after transport fail.
      queuePending(item);
      pump();
    }, delay);
  }

  function pump() {
    try {
      pruneStaleInflight();
    } catch (ePr) {}
    var conc = effectiveUploadCap();
    if (conc <= 0) return;
    // After halt, still pump B10x / force_after_halt (EDH must-land).
    if (
      haltState &&
      !pending.some(function (p) {
        return allowDespiteHalt(p);
      })
    ) {
      return;
    }
    // Never start more than conc slots even if counter drifted.
    if (inflight > conc) inflight = Math.min(inflight, inflightItems.length, conc);
    while (inflight < conc && pending.length) {
      pending.sort(function (a, b) {
        return effectivePriority(b) - effectivePriority(a);
      });
      var item = pending.shift();
      if (uploadsBlocked(item)) {
        haltedDrops++;
        continue;
      }
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          // Soft quiet budget: drop soft packs, keep hard.
          if (!isHardAnchorBatch(item.batch_id)) {
            failed++;
            continue;
          }
        }
      } catch (eQ) {}
      // Deferred retry: not ready yet.
      if (item._retry_after_ms && item._retry_after_ms > Date.now()) {
        queuePending(item);
        break;
      }
      // P1: serialize heavy (B10x/B7) — max 1 inflight to cut Failed to fetch storms.
      var itemHeavy = isHeavyBatch(item.batch_id);
      if (itemHeavy && heavyInflight >= heavyMaxInflight()) {
        // Prefer a light pack next; requeue this heavy for later.
        var lightIdx = -1;
        for (var hi = 0; hi < pending.length; hi++) {
          if (!isHeavyBatch(pending[hi].batch_id)) {
            lightIdx = hi;
            break;
          }
        }
        if (lightIdx >= 0) {
          var light = pending.splice(lightIdx, 1)[0];
          queuePending(item);
          item = light;
          itemHeavy = false;
        } else {
          pending.unshift(item);
          break;
        }
      }
      var k = keyOf(item);
      attempts[k] = (attempts[k] || 0) + 1;
      if (hiding && isHardAnchorBatch(item.batch_id) && attempts[k] > 1) {
        hardRetries++;
      }
      inflight++;
      inflightItems.push(item);
      registryUpsertFromItem(item, "uploading");
      if (itemHeavy) heavyInflight++;
      // iss P0: bind each in-flight upload to its own item/k/heavy — never close over
      // function-scoped `var` from the pump while-loop (concurrent ACK mis-attribution).
      startUpload(item, itemHeavy, k);
    }
  }

  function startUpload(item, itemHeavy, k) {
      postOne(item)
        .then(function (resp) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          if (haltState) {
            pump();
            return;
          }
          sent++;
          if (isHardAnchorBatch(item.batch_id)) hardFlushed++;
          // iss/72 P0-4: only verified ACK → sentKeys / batches_ok.
          try {
            var ackRes = applyAck(item, resp || {});
            var ackT = ackRes && typeof ackRes === "object" ? ackRes.type : ackRes;
            var verified = ackRes && typeof ackRes === "object" ? !!ackRes.verified : false;
            if (ackT === "conflict") {
              try {
                if (global.GROps && GROps.report) {
                  GROps.report(
                    "upload_ack_conflict",
                    "upload",
                    {
                      batch_id: item.batch_id,
                      generation: item.material_generation,
                      payload_hash: item.payload_hash,
                    },
                    "warn"
                  );
                }
              } catch (eCf) {}
            } else if (verified) {
              sentKeys[k] = true;
              try {
                sentKeys[k + "|ts"] = Date.now();
              } catch (eTs) {}
              if (resp && resp.sealed === false) noteOutcome("plain_ok", item.batch_id, item);
              else noteOutcome("sealed_ok", item.batch_id, item);
              registryUpsertFromItem(item, "acked");
            } else {
              // transport_ok_unverified — do not short-circuit future retries incorrectly
              registryUpsertFromItem(item, "transport_ok_unverified");
            }
          } catch (eOut) {}
          try {
            aimdOnSuccess();
          } catch (eAim) {}
          // Mild ramp after first success — never exceed concurrency_max.
          if (!ramped && cfg.ramp_after_first) {
            ramped = true;
            var maxC = Number(cfg.concurrency_max) || 6;
            var rampT = Math.min(maxC, Number(cfg.ramp_after_first) || 4);
            if (cfg.concurrency < rampT) cfg.concurrency = rampT;
          }
          // Success-path terminal signals (complete / cool) without waiting for 410.
          var termOk = parseTerminal(200, resp || {});
          if (termOk.terminal) {
            applyHalt(termOk.code || "probe_complete", sessionOf(item), termOk.code);
          } else {
            maybeLocalIdentityDone();
          }
          try {
            // Keep client-side received inventory for hard-SLA (no cache-clear required).
            global.__GR_RECEIVED_BATCHES__ = global.__GR_RECEIVED_BATCHES__ || [];
            global.__GR_RECEIVED_BATCHES__.push({
              batch_id: item.batch_id,
              source: item.source || "main",
            });
            global.dispatchEvent(
              new CustomEvent("gr-upload-ok", {
                detail: {
                  batch_id: item.batch_id,
                  source: item.source || "main",
                  response: resp || null,
                  cycle_probe_status: (resp && resp.cycle_probe_status) || null,
                },
              })
            );
          } catch (eEv) {}
          pump();
        })
        .catch(function (err) {
          if (itemHeavy) heavyInflight = Math.max(0, heavyInflight - 1);
          // removeInflightItem owns inflight counter (do not inflight-- here).
          removeInflightItem(item);
          try {
            if (err && (err.seal_required || err.status === 426)) noteOutcome("sealed_reject", item.batch_id);
            else if (err && err.network) noteOutcome("network", item.batch_id);
            else if (err && !err.silent) noteOutcome("other_fail", item.batch_id);
          } catch (eOut2) {}
          try {
            if (!err || !err.silent) aimdOnFail();
          } catch (eAimF) {}
          try {
            registryUpsertFromItem(item, "retry_wait");
          } catch (eReg) {}
          // Terminal: cycle done / cool / session_expired — never retry flood.
          if (err && err.terminal) {
            applyHalt(
              err.code || "session_expired",
              sessionOf(item),
              err.code || "session_expired"
            );
            if (!err.silent) failed++;
            else haltedDrops++;
            try {
              if (!err.silent) {
                global.dispatchEvent(
                  new CustomEvent("gr-upload-terminal", {
                    detail: {
                      batch_id: item.batch_id,
                      status: err.status,
                      code: err.code,
                      error: String(err.message || err),
                    },
                  })
                );
              }
            } catch (eT) {}
            // Do not pump after terminal — queue drained in applyHalt.
            return;
          }
          if (uploadsBlocked(item) || haltState) {
            haltedDrops++;
            return;
          }
          try {
            if (!err || !err.silent) {
              global.dispatchEvent(
                new CustomEvent("gr-upload-fail", {
                  detail: {
                    batch_id: item.batch_id,
                    pack_id: item.pack_id || item.batch_id,
                    status: err && err.status,
                    code: err && err.code,
                    network: !!(err && err.network),
                    error: String((err && err.message) || err || ""),
                  },
                })
              );
            }
          } catch (eFailEv) {}
          // Hard / deepen / seal fail: clear sentKeys so re-upload can land (never stick "sent").
          try {
            var bidR = String(item.batch_id || "");
            var retriableFail =
              (err && err.network) ||
              (err && err.cf_challenge) ||
              (err &&
                (err.code === "seal_failed" ||
                  err.code === "sealed_required" ||
                  err.seal_failed ||
                  err.seal_required)) ||
              (err && err.timeout_abort) ||
              (err && err.status === 426) ||
              // Edge 403/503 (CF under-attack / rate) — retry hard/deepen anchors.
              (err &&
                (err.status === 403 || err.status === 503 || err.status === 429) &&
                (isHardAnchorBatch(bidR) || isDeepenBatch(bidR)));
            if (retriableFail) {
              // Always clear dedupe on seal/426 so sealed retry can land any batch.
              if (
                err.seal_failed ||
                err.seal_required ||
                err.status === 426 ||
                isHardAnchorBatch(bidR) ||
                isDeepenBatch(bidR) ||
                bidR === "B27_storage_privacy" ||
                bidR === "B65_client_hints_full" ||
                bidR === "B18_webgpu" ||
                bidR === "B46_audio_deep" ||
                bidR === "B55_webrtc_ice_deep" ||
                bidR === "B2_hardware" ||
                bidR === "B3_system" ||
                bidR === "B0_bootstrap" ||
                bidR === "B1_conflict" ||
                bidR === "B11_interaction" ||
                bidR === "B12_anti_camouflage" ||
                bidR === "B7_sandbox"
              ) {
                delete sentKeys[k];
              }
            }
          } catch (eSk) {}
          // seal_failed / 426 is never cycle-terminal — always retry path above.
          var isSealRace =
            err &&
            (err.code === "seal_failed" ||
              err.code === "sealed_required" ||
              err.seal_failed ||
              err.seal_required ||
              err.status === 426 ||
              /seal_grant|seal_timeout|seal_module/i.test(String((err && err.message) || "")));
          if (isSealRace) {
            try {
              err.terminal = false;
            } catch (eTerm) {}
            try {
              sealCircuitTrip(err && (err.code || err.message));
            } catch (eCt) {}
            // Permanent WASM failure: limited seal waits then stop (no 70× storm).
            try {
              var wasmDeadQ =
                !!(err && err.seal_wasm_dead) ||
                !!global.__GR_SEAL_WASM_DEAD__ ||
                /seal_wasm_unsupported|wasm_not_binary|seal_wasm_required/i.test(
                  String((err && err.message) || "")
                );
              item._seal_waits = (item._seal_waits || 0) + 1;
              if (wasmDeadQ && item._seal_waits >= 2) {
                err.terminal = true;
                err.code = err.code || "seal_wasm_dead";
                // Do not decrement attempt — count as real fail.
              } else if ((attempts[k] || 0) > 0) {
                // Do not burn hard-attempt budget on open-lag seal races.
                attempts[k] = Math.max(0, attempts[k] - 1);
              }
              // Circuit open: defer retries longer (transport-only, no re-collect).
              if (sealCircuitOpen()) {
                item._retry_after_ms = Date.now() + 8000;
              }
            } catch (eDec) {}
            // Align session with grant before retry (open supersede).
            try {
              var fixSid =
                (global.__GR_SESSION_ID__ ||
                  (global.GRSeal &&
                    global.GRSeal.resolveSealSessionId &&
                    GRSeal.resolveSealSessionId(item.session_id)) ||
                  cfg.session_id ||
                  "") + "";
              if (fixSid && String(fixSid).indexOf("cycle_") === 0) {
                item.session_id = fixSid;
              }
            } catch (eFix) {}
            // Remint grant in parallel (coalesced).
            try {
              refreshSealGrant(sessionOf(item));
            } catch (eRf) {}
          }
          var cap = maxAttemptsForItem(item);
          var nTry = attempts[k] || 1;
          var sealWaits = item._seal_waits || 0;
          // Seal race: large seal_wait budget; real attempts still capped separately.
          if (isSealRace) {
            cap = Math.max(cap, isHardAnchorBatch(item.batch_id) || isB10xBatch(item.batch_id) ? 14 : 10);
            // Allow continue while seal waits < 16 even if attempt counter looks high.
            if (sealWaits < 16 && nTry >= cap) {
              nTry = cap - 1;
              attempts[k] = nTry;
            }
          }
          if (nTry < cap || (isSealRace && sealWaits < 16)) {
            if (isHardAnchorBatch(item.batch_id) && !isSealRace) hardRetries++;
            if ((item.priority || 0) >= 70) {
              item.priority = Math.min(100, (item.priority || 70) + 1);
            }
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail && !isSealRace) {
                GRProbeLifecycle.recordFail(
                  item.batch_id,
                  (err && (err.code || (err.timeout_abort && "upload_timeout_abort") || err.message)) ||
                    "upload_fail"
                );
              }
            } catch (eLf) {}
            // Seal grant race: wait for remint / open grant (not exponential storm).
            if (isSealRace) {
              var sealDelay = Math.min(4000, 300 + sealWaits * 400);
              item._retry_after_ms = Date.now() + sealDelay;
              queuePending(item);
              setTimeout(function () {
                try {
                  if (item._retry_after_ms && item._retry_after_ms <= Date.now()) {
                    delete item._retry_after_ms;
                  }
                } catch (eClr) {}
                pump();
              }, sealDelay + 20);
              pump();
            } else {
              // Delayed requeue — avoid spin retry storms.
              scheduleRetry(item, nTry);
              pump();
            }
          } else {
            failed++;
            try {
              if (global.GRProbeLifecycle && GRProbeLifecycle.recordFail) {
                var fr = GRProbeLifecycle.recordFail(item.batch_id, "attempts_exhausted");
                // Dedupe exhausted reports per batch (was flooding ops).
                var exKey = "ex:" + String(item.batch_id || "");
                global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                var lastEx = global.__GR_EXHAUST_REPORT__[exKey] || 0;
                var nowEx = Date.now();
                if (nowEx - lastEx > 30000 && global.GROps && GROps.report) {
                  global.__GR_EXHAUST_REPORT__[exKey] = nowEx;
                  // Commercial hard only → error; B10x/B47 network exhaust → warn (v150 fix).
                  var commercialHard = isCommercialHardForOps(item.batch_id);
                  var deepenish =
                    isDeepenBatch(item.batch_id) ||
                    isHeavyBatch(item.batch_id) && !commercialHard;
                  var exCode = commercialHard
                    ? "upload_hard_exhausted"
                    : deepenish
                      ? "upload_deepen_exhausted"
                      : "upload_soft_exhausted";
                  var exSev = commercialHard ? "error" : "warn";
                  // Partial success: B10 already landed + only deepen failed → always warn.
                  try {
                    if (
                      !commercialHard &&
                      sessionOutcome.batches_ok &&
                      sessionOutcome.batches_ok["B10_hw_curves"]
                    ) {
                      exSev = "warn";
                    }
                  } catch (ePart) {}
                  var apiHost = "";
                  try {
                    var ab = String(cfg.apiBase || global.__GR_API_BASE__ || "/g5");
                    apiHost = ab.indexOf("http") === 0 ? ab.split("/").slice(0, 3).join("/") : ab;
                  } catch (eH) {}
                  // Ops dimensions: attach last net_class for this batch (if any).
                  var lastNc = null;
                  var lastTransport = null;
                  try {
                    var netHist = global.__GR_NET_CLASS_LAST__ || Object.create(null);
                    var nk = String(item.batch_id || "");
                    if (netHist[nk]) {
                      lastNc = netHist[nk].net_class || null;
                      lastTransport = netHist[nk].transport || null;
                    }
                  } catch (eNc) {}
                  GROps.report(
                    exCode,
                    "upload",
                    {
                      batch_id: item.batch_id,
                      attempts: nTry,
                      over_budget: !!(fr && fr.overBudget),
                      commercial_hard: commercialHard,
                      deepen: !!deepenish,
                      b10_already_ok: !!(
                        sessionOutcome.batches_ok &&
                        sessionOutcome.batches_ok["B10_hw_curves"]
                      ),
                      api_base: apiHost,
                      online:
                        typeof navigator !== "undefined" && navigator.onLine != null
                          ? !!navigator.onLine
                          : null,
                      // taxonomy dimensions (server also normalizes)
                      net_class: lastNc || undefined,
                      transport: lastTransport || undefined,
                      fail_class_hint: commercialHard
                        ? "hard_exhaust"
                        : deepenish
                          ? "deepen_exhaust"
                          : "soft_exhaust",
                      impact_band_hint: commercialHard
                        ? "commercial_critical"
                        : "deepen_optional",
                    },
                    exSev
                  );
                }
              }
            } catch (eEx) {}
            pump();
          }
        });
  }

  var api = {
    configure: function (o) {
      o = o || {};
      var prevSid = cfg.session_id;
      Object.keys(o).forEach(function (k) {
        if (o[k] !== undefined) cfg[k] = o[k];
      });
      // New session id → isolate queue/registry (iss/72 P1-2).
      if (o.session_id && String(o.session_id) !== String(prevSid || "")) {
        if (haltState && haltState.session_id !== String(o.session_id)) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
          } catch (eC) {}
        }
        resetSessionState("configure_session_id");
      }
      // First-party /g5: raise caps only within min/max budget.
      applyFirstPartyPerfHints();
    },
    /** Mark keys already retained server-side (from open/session received list). */
    seedSentKeys: function (keys) {
      (keys || []).forEach(function (k) {
        if (typeof k === "string") sentKeys[k] = true;
        else if (k && k.batch_id) {
          sentKeys[keyOf(k)] = true;
        }
      });
    },
    /**
     * Clear dedupe keys so force_recollect can re-upload the same batch
     * (e.g. B2 present without residual → brain requests recollect).
     */
    clearSentKeys: function (items) {
      (items || []).forEach(function (it) {
        if (typeof it === "string") {
          delete sentKeys[it];
        } else if (it && it.batch_id) {
          var bidClr = String(it.batch_id);
          // Budget: after terminal ok, only one force_recollect clear per session.
          if (sessionOutcome.batches_ok[bidClr] && forceRecollectUsed[bidClr]) {
            return;
          }
          if (sessionOutcome.batches_ok[bidClr]) {
            forceRecollectUsed[bidClr] = 1;
          }
          delete sentKeys[keyOf(it)];
          try {
            delete sessionOutcome.batches_ok[bidClr];
            delete sessionOutcome.batches_started[bidClr];
          } catch (eOk) {}
        }
      });
    },
    /**
     * Whether brain force_recollect may clear + re-upload this batch.
     * Terminal ok batches: at most once per session.
     */
    allowForceRecollect: function (batchId) {
      var bid = String(batchId || "");
      if (!bid) return false;
      if (!sessionOutcome.batches_ok[bid]) return true;
      if (forceRecollectUsed[bid]) return false;
      forceRecollectUsed[bid] = 1;
      return true;
    },
    enqueue: function (item) {
      if (!item || !item.batch_id) return;
      var bidEnq = String(item.batch_id || "");
      var recollect =
        item.force_recollect === true ||
        item.forceRecollect === true ||
        item.reset_attempts === true;
      // New material generation on explicit recollect.
      if (recollect) {
        try {
          item._capture_frozen = false;
          item.material_generation = (item.material_generation || 1) + 1;
          item.capture_id = null;
        } catch (eRc) {}
      }
      // Success short-circuit: already sealed_ok → skip unless brain force_recollect.
      // (Was: B10x/secondary always force=true → 5–10× wire storms, iss/65.)
      if (batchAlreadyOk(bidEnq) && !recollect) {
        // Explicit force without recollect still blocked once ok (start heartbeat spam).
        successShortCircuit++;
        skipped++;
        return;
      }
      // pagehide: reject non-allowlisted packs immediately.
      if ((hiding || isPageUnloading()) && !isPagehideAllowlisted(bidEnq) && !recollect) {
        pagehideDropped++;
        skipped++;
        return;
      }
      // B10x EDH: force only until first sealed_ok (or recollect). Survive halt/cool.
      if (isB10xBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Secondary silicon/infra: same — once landed, no force storm.
      if (isSecondaryInfraBatch(bidEnq)) {
        if (!batchAlreadyOk(bidEnq) || recollect) {
          item.force = true;
          item.force_after_halt = true;
          item.allow_during_stop = true;
        }
      }
      // Hard/deepen attempt ceiling across SLA re-kicks (prevents attempts:16 storms).
      try {
        var kCap = keyOf(item);
        var capEnq = maxAttemptsForItem(item);
        // B10 / B10x / seal remint: higher ceiling + grant-ready reset (short-visit residual).
        if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if (item.reset_attempts) {
          delete attempts[kCap];
          delete item._seal_waits;
        }
        // If grant is now valid after seal race, clear fake exhaustion and continue.
        try {
          if (
            (attempts[kCap] || 0) >= capEnq &&
            global.GRSeal &&
            GRSeal.grantRawValid &&
            GRSeal.grantRawValid()
          ) {
            attempts[kCap] = Math.max(0, capEnq - 4);
            item._seal_waits = 0;
          }
        } catch (eGr) {}
        // Secondary infra always gets elevated attempt cap
        if (isSecondaryInfraBatch(item.batch_id)) {
          capEnq = Math.max(capEnq, 16);
        }
        if ((attempts[kCap] || 0) >= capEnq && !item.reset_attempts) {
          // Soft re-arm hard/B10x every 8s instead of permanent drop.
          if (isB10xBatch(item.batch_id) || isHardAnchorBatch(item.batch_id) || isSecondaryInfraBatch(item.batch_id)) {
            var lastCap = item._cap_block_ms || 0;
            var rearmMs = isHardAnchorBatch(item.batch_id) && !isB10xBatch(item.batch_id) ? 6000 : 8000;
            if (Date.now() - lastCap > rearmMs) {
              item._cap_block_ms = Date.now();
              attempts[kCap] = Math.max(0, capEnq - 5);
              item._seal_waits = 0;
              try {
                refreshSealGrant(sessionOf(item));
              } catch (eR2) {}
            } else {
              skipped++;
              try {
                if (global.GROps && GROps.report) {
                  var exK = "excap:" + String(item.batch_id || "");
                  global.__GR_EXHAUST_REPORT__ = global.__GR_EXHAUST_REPORT__ || Object.create(null);
                  var nowC = Date.now();
                  if (nowC - (global.__GR_EXHAUST_REPORT__[exK] || 0) > 60000) {
                    global.__GR_EXHAUST_REPORT__[exK] = nowC;
                    GROps.report(
                      "upload_hard_exhausted",
                      "upload",
                      {
                        batch_id: item.batch_id,
                        attempts: attempts[kCap],
                        reason: "enqueue_cap",
                        rearm_ms: rearmMs,
                      },
                      "warn"
                    );
                  }
                }
              } catch (eCap) {}
              return;
            }
          } else {
            skipped++;
            return;
          }
        }
      } catch (eEnqCap) {}
      // Hard gate: after 410/complete never enqueue (ignore force — force only skips dedupe),
      // except B10x / force_after_halt.
      if (uploadsBlocked(item) && !allowDespiteHalt(item)) {
        haltedDrops++;
        skipped++;
        return;
      }
      try {
        if (
          (global.__GR_STOP_PROBE__ || global.__GR_HALT_UPLOADS__ || global.__GR_CYCLE_CLOSED__) &&
          !allowDespiteHalt(item)
        ) {
          haltedDrops++;
          skipped++;
          return;
        }
      } catch (eStop) {}
      try {
        if (
          global.GRProbeLifecycle &&
          GRProbeLifecycle.allowEnqueue &&
          !GRProbeLifecycle.allowEnqueue(item.batch_id)
        ) {
          if (!isHardAnchorBatch(item.batch_id)) {
            skipped++;
            return;
          }
        }
      } catch (eAe) {}
      var k = keyOf(item);
      // RPA continuous: never force-bypass queue coalescing (gecko flood).
      var rpa = isRpaBatch(item.batch_id);
      // Terminal success short-circuit (not start heartbeat). force alone is not enough.
      if (batchAlreadyOk(bidEnq) && !recollect) {
        successShortCircuit++;
        skipped++;
        return;
      }
      // sentKeys: allow one upgrade from start→done (started marked sentKeys but not batches_ok).
      if (sentKeys[k] && !item.force && !recollect && batchAlreadyOk(bidEnq)) {
        skipped++;
        return;
      }
      if (sentKeys[k] && !item.force && !recollect && !isStartHeartbeatItem(item)) {
        // Final payload after start: need force or clear. midEnqueue sets force when !alreadySent;
        // after start alreadySent is true — allow if only batches_started (not batches_ok).
        if (!sessionOutcome.batches_started[bidEnq] || sessionOutcome.batches_ok[bidEnq]) {
          skipped++;
          return;
        }
        // Upgrade path: start was sent, final multipath ready — allow once.
        item.force = true;
      }
      // For RPA, even force: coalesce; never on unload (pagehide drops B11).
      if (rpa && (hiding || isPageUnloading())) {
        pagehideDropped++;
        skipped++;
        return;
      }
      if (rpa && sentKeys[k] && !item.pagehide_flush && !recollect) {
        // Allow re-upload only after quiet window (30s) — was 5s and still noisy.
        var lastOk = sentKeys[k + "|ts"] || 0;
        if (Date.now() - lastOk < 30000) {
          skipped++;
          return;
        }
      }
      // Freeze only after all dedupe, lifecycle, pagehide, and terminal gates.
      // Self-heal may replay many descriptors after cycle completion; dropped
      // items must not pay the deep-clone/hash cost or inflate capture metrics.
      try {
        if (item.payload) freezeCapture(item);
      } catch (eFzEnq) {}
      // Dedup pending by session|batch|source — including force (dense hedge / secondary).
      // Previous: force=true skipped replace → same B19 could stack dozens of times.
      queuePending(item);
      pump();
    },
    /**
     * When open returns a different cycle id than the client mint (completed sticky
     * bag superseded), retarget pending items so they land on the live cycle.
     */
    rewriteSessionId: function (fromId, toId) {
      var from = String(fromId || "");
      var to = String(toId || "");
      if (!from || !to || from === to) return 0;
      var n = 0;
      for (var i = 0; i < pending.length; i++) {
        var it = pending[i];
        if (!it) continue;
        var sid = String(it.session_id || cfg.session_id || "");
        if (sid === from || !it.session_id) {
          it.session_id = to;
          n++;
        }
      }
      try {
        cfg.session_id = to;
      } catch (eC) {}
      // Drop sentKeys bound to the dead cycle so re-upload of same batch is allowed.
      try {
        Object.keys(sentKeys).forEach(function (k) {
          if (String(k).indexOf(from + "|") === 0) delete sentKeys[k];
        });
      } catch (eK) {}
      // If we halted only because of 410 on the dead cycle, allow the live cycle to proceed.
      if (haltState && String(haltState.session_id || "") === from) {
        haltState = null;
        try {
          global.__GR_HALT_UPLOADS__ = false;
          global.__GR_STOP_PROBE__ = false;
          global.__GR_SKIP_IDENTITY__ = false;
          global.__GR_CYCLE_CLOSED__ = null;
          global.__GR_UPLOAD_HALT__ = null;
          global.__GR_PHASE__ = "active";
          if (!cfg.concurrency || cfg.concurrency < 1) cfg.concurrency = 3;
          // Undo false local cool stamped by 410 on the superseded bag.
          var S = global.GRStorage;
          if (S && S.setCoolUntil) S.setCoolUntil(0);
        } catch (eR) {}
        pump();
      }
      // Open supersede often arrives with seal_grant for `to` — wake seal waiters.
      pump();
      return n;
    },
    /** Wake pump after open.seal_grant / hot-swap re-eval (B0 may be grant-waiting). */
    kick: function () {
      try {
        if (global.GRSeal && GRSeal.adoptFromGlobals) GRSeal.adoptFromGlobals();
      } catch (eK) {}
      // Clear stale seal retry gates so grant-ready items run immediately.
      try {
        for (var i = 0; i < pending.length; i++) {
          if (pending[i] && pending[i]._retry_after_ms) {
            delete pending[i]._retry_after_ms;
          }
        }
      } catch (eR) {}
      pump();
    },
    /** Stop identity uploads for a cycle (cool open / 410 complete). */
    halt: function (reason, sessionId, code) {
      applyHalt(reason || "halt", sessionId, code);
      return haltState;
    },
    isHalted: function (sessionId) {
      if (!haltState) return false;
      if (sessionId == null || sessionId === "") return true;
      return !haltState.session_id || haltState.session_id === String(sessionId);
    },
    haltState: function () {
      return haltState ? Object.assign({}, haltState) : null;
    },
    /** Snapshot of FE local view for multi-party reconcile with BE. */
    clientView: function () {
      var sid = cfg.session_id || global.__GR_SESSION_ID__ || "";
      var sent = [];
      try {
        Object.keys(sentKeys).forEach(function (k) {
          var parts = String(k).split("|");
          if (parts.length >= 2) sent.push(parts[1]);
        });
      } catch (eS) {}
      return {
        session_id: sid,
        stop_probe: !!global.__GR_STOP_PROBE__,
        skip_identity: !!global.__GR_SKIP_IDENTITY__,
        halted: !!haltState,
        phase: global.__GR_PHASE__ || null,
        local_uploads_done: !!(
          global.__GR_IDENTITY_UPLOADS_DONE__ &&
          global.__GR_IDENTITY_UPLOADS_DONE__.session_id === sid
        ),
        sent_batch_ids: sent,
        last_http_status: (haltState && haltState.status) || null,
        last_error_code: (haltState && haltState.code) || null,
      };
    },
    /**
     * Apply BE reconcile corrections (server authoritative for cycle lifecycle).
     * Returns applied action names.
     */
    applyCorrections: function (reconcileBody) {
      var applied = [];
      if (!reconcileBody) return applied;
      var sid =
        (reconcileBody.session_id ||
          cfg.session_id ||
          global.__GR_SESSION_ID__ ||
          "") + "";
      try {
        if (reconcileBody.cycle_probe_status) {
          global.__GR_CYCLE_PROBE_STATUS__ = reconcileBody.cycle_probe_status;
        }
        if (reconcileBody.business_state) {
          global.__GR_BUSINESS_STATE__ = reconcileBody.business_state;
        }
      } catch (eG) {}
      var list = reconcileBody.corrections || [];
      for (var i = 0; i < list.length; i++) {
        var c = list[i] || {};
        var act = c.action || "";
        if (act === "fe_halt_uploads" || reconcileBody.fe_should_halt) {
          // Refuse cool-halt while primary residual not acked and still in flight/pending.
          // Server may claim identity_complete_cool from B10x-only has_b10 false positive.
          var refuseHalt = false;
          try {
            var hasPrimary =
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["B10_hw_curves"]) ||
              !!(sessionOutcome.batches_ok && sessionOutcome.batches_ok["mid.curves"]);
            var hardPending = false;
            var pi;
            for (pi = 0; pi < pending.length; pi++) {
              if (isHardAnchorBatch(pending[pi] && pending[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            for (pi = 0; !hardPending && pi < inflightItems.length; pi++) {
              if (isHardAnchorBatch(inflightItems[pi] && inflightItems[pi].batch_id)) {
                hardPending = true;
                break;
              }
            }
            var reasonStr = String((c && c.reason) || reconcileBody.business_state || "");
            if (
              !hasPrimary &&
              (hardPending || reasonStr.indexOf("identity_complete") >= 0 || reasonStr.indexOf("cycle_status=complete") >= 0)
            ) {
              refuseHalt = true;
            }
          } catch (eRef) {}
          if (refuseHalt) {
            applied.push("fe_halt_uploads_refused_missing_b10");
          } else {
            applyHalt(c.reason || "reconcile_halt", sid, c.reason || "reconcile");
            applied.push("fe_halt_uploads");
          }
        } else if (
          act === "fe_resume_if_same_active_cycle" ||
          act === "fe_clear_false_cool" ||
          reconcileBody.fe_should_resume
        ) {
          haltState = null;
          try {
            global.__GR_UPLOAD_HALT__ = null;
            global.__GR_CYCLE_CLOSED__ = null;
            global.__GR_STOP_PROBE__ = false;
            global.__GR_SKIP_IDENTITY__ = false;
            global.__GR_PHASE__ = "active";
          } catch (eR) {}
          applied.push(act || "fe_resume");
        } else if (act === "fe_mark_local_done_only") {
          maybeLocalIdentityDone();
          applied.push(act);
        } else if (act === "rebind_session_id") {
          applied.push(act);
        } else if (act === "be_completed_cycle" || act === "be_complete_cycle") {
          applyHalt("be_complete", sid, "cycle_complete");
          applied.push(act);
        }
      }
      if (reconcileBody.fe_should_halt && !haltState) {
        applyHalt("reconcile_halt", sid, reconcileBody.business_state || "halt");
        applied.push("fe_should_halt");
      }
      if (reconcileBody.aligned && reconcileBody.halt_uploads && !haltState) {
        applyHalt("aligned_halt", sid, reconcileBody.business_state || "halt");
        applied.push("aligned_halt");
      }
      try {
        global.dispatchEvent(
          new CustomEvent("gr-status-reconciled", {
            detail: {
              applied: applied,
              body: reconcileBody,
              session_id: sid,
            },
          })
        );
      } catch (eE) {}
      return applied;
    },
    /** New page cycle: clear halt so a fresh session_id can upload. */
    resume: function (newSessionId) {
      if (newSessionId && haltState && haltState.session_id === String(newSessionId)) {
        // Same id still closed.
        return haltState;
      }
      haltState = null;
      // Restore pump capacity for a new cycle.
      if (!cfg.concurrency || cfg.concurrency < 1) {
        cfg.concurrency = 3;
        ramped = false;
      }
      try {
        global.__GR_UPLOAD_HALT__ = null;
        global.__GR_CYCLE_CLOSED__ = null;
        global.__GR_HALT_UPLOADS__ = false;
        // Do not clear STOP_PROBE here — boot decides for cool windows.
      } catch (eR) {}
      return null;
    },
    /** B0 enqueued: allow parallel rest while first upload still inflight. */
    softRamp: function (n) {
      var maxC = Number(cfg.concurrency_max) || 6;
      var target = Math.min(maxC, Number(n) || cfg.mid_ramp_concurrency || 4);
      if (!ramped && cfg.concurrency < target) {
        cfg.concurrency = target;
      }
      pump();
    },
    /**
     * Flush pending uploads.
     * - pagehide / beforeunload / unload → true unload (PAGE_HIDING, lower hide attempts)
     * - hidden / background → tab not focused; keep alive retries; do NOT stop probe
     */
    flush: function (reason) {
      flushReason = reason || "flush";
      var r = String(reason || "");
      var unloading =
        r === "pagehide" || r === "beforeunload" || r === "unload" || r === "close";
      var background = r === "hidden" || r === "background" || r === "visibility_hidden";
      if (unloading) {
        hiding = true;
        try {
          global.__GR_PAGE_HIDING__ = true;
          global.__GR_PAGE_UNLOADING__ = true;
        } catch (e) {}
        // Abort non-allowlisted inflight so keepalive slots go to hard/B10x (iss/65).
        try {
          var keptCtrls = [];
          for (var ai = 0; ai < inflightCtrls.length; ai++) {
            var c = inflightCtrls[ai];
            var cBid = c && c.__gr_batch_id ? String(c.__gr_batch_id) : "";
            if (cBid && isPagehideAllowlisted(cBid)) {
              keptCtrls.push(c);
              continue;
            }
            try {
              if (c && typeof c.abort === "function") {
                c.__gr_soft_unload_abort = 1;
                c.abort();
                softAbortOnUnload++;
              }
            } catch (eAb) {}
          }
          inflightCtrls = keptCtrls;
        } catch (eAc) {}
        // Drop pending non-allowlisted (mid/R/B11) — boost alone is not enough.
        try {
          var kept = [];
          for (var pi = 0; pi < pending.length; pi++) {
            var pit = pending[pi];
            if (!pit) continue;
            if (isPagehideAllowlisted(pit.batch_id) && !batchAlreadyOk(pit.batch_id)) {
              kept.push(pit);
            } else {
              pagehideDropped++;
            }
          }
          pending = kept;
        } catch (eDrop) {}
        // Keepalive window is tiny — max concurrency for remaining hard/B10x only.
        try {
          var maxC = Number(cfg.concurrency_max) || 6;
          cfg.concurrency = Math.max(Number(cfg.concurrency) || 3, Math.min(maxC, 6));
        } catch (eConc) {}
      } else if (background) {
        // iss/70: background lowers budget; do not maximize.
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
          global.__GR_PAGE_HIDING__ = false;
          // Cap concurrency while backgrounded.
          cfg.concurrency = Math.min(Number(cfg.concurrency) || 3, 2);
        } catch (eBg) {}
      }
      // Boost hard anchors still pending so flush prioritizes commercial materials.
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id) || isDeepenBatch(pending[i].batch_id)) {
          pending[i].priority =
            (pending[i].priority || 0) +
            (isHardAnchorBatch(pending[i].batch_id)
              ? cfg.hard_anchor_priority_boost || 1000
              : 200);
          pending[i].hard_anchor_flush = isHardAnchorBatch(pending[i].batch_id);
        }
      }
      pump();
      return api.stats();
    },
    /** Tab visible again — restore full concurrency and clear background flag. */
    markVisible: function () {
      hiding = false;
      try {
        global.__GR_PAGE_BACKGROUNDED__ = false;
        // Never clear true unload flags here (page is going away).
        if (!global.__GR_PAGE_UNLOADING__) {
          global.__GR_PAGE_HIDING__ = false;
        }
      } catch (eV) {}
      applyFirstPartyPerfHints();
      if (!ramped && cfg.concurrency < (cfg.mid_ramp_concurrency || 8)) {
        cfg.concurrency = Math.max(cfg.concurrency, cfg.mid_ramp_concurrency || 8);
      }
      pump();
    },
    markHiding: function () {
      // Legacy: treat as background unless already unloading.
      if (!isPageUnloading()) {
        hiding = false;
        try {
          global.__GR_PAGE_BACKGROUNDED__ = true;
        } catch (eM) {}
      } else {
        hiding = true;
      }
    },
    isHardAnchorBatch: isHardAnchorBatch,
    isDeepenBatch: isDeepenBatch,
    isCommercialLandFast: isCommercialLandFast,
    backoffMsForItem: backoffMsForItem,
    isPageUnloading: isPageUnloading,
    isPageBackgrounded: isPageBackgrounded,
    /** Pure: commercial/hard final complete (exported for boot multi-tick). */
    hardFinalComplete: hardFinalComplete,
    parseTerminal: parseTerminal,
    alreadySent: function (item) {
      return !!sentKeys[keyOf(item || {})];
    },
    /** iss/70 P0: batch has frozen capture pending upload/retry (hard-SLA must not re-collect). */
    hasPendingCapture: hasPendingCapture,
    /** iss/70: acked | conflict | uploading | retry_wait | queued | collected | absent */
    materialState: materialState,
    /**
     * Self-heal Loop T: re-pump pending or re-queue frozen capture for a batch.
     * Does not re-collect GPU materials.
     */
    nudgeTransport: function (batchId, sessionId) {
      var bid = String(batchId || "");
      if (!bid) {
        pump();
        return false;
      }
      var sidN = sessionId != null ? String(sessionId) : String(cfg.session_id || "");
      var i;
      // Clear deferred retry so pump can take it now.
      for (i = 0; i < pending.length; i++) {
        if (String(pending[i].batch_id || "") !== bid) continue;
        if (sidN && pending[i].session_id && String(pending[i].session_id) !== sidN) continue;
        try {
          delete pending[i]._retry_after_ms;
        } catch (eClr) {}
      }
      pump();
      return hasPendingCapture(bid, sidN);
    },
    /** Force pump (self-heal / tests). */
    pump: pump,
    freezeCapture: freezeCapture,
    payloadHashOf: payloadHashOf,
    applyAck: applyAck,
    registryGet: registryGet,
    registrySnapshot: function () {
      var out = {};
      Object.keys(materialRegistry).forEach(function (k) {
        out[k] = materialRegistry[k];
      });
      return out;
    },
    effectiveUploadCap: effectiveUploadCap,
    stats: function () {
      var hardPending = 0;
      var pendingBatches = [];
      var seenB = {};
      for (var i = 0; i < pending.length; i++) {
        if (isHardAnchorBatch(pending[i].batch_id)) hardPending++;
        var pb = String((pending[i] && pending[i].batch_id) || "");
        if (pb && !seenB[pb]) {
          seenB[pb] = 1;
          pendingBatches.push(pb);
        }
      }
      return {
        pending: pending.length,
        pending_batches: pendingBatches.slice(0, 32),
        inflight: inflight,
        inflight_items: inflightItems.length,
        concurrency: cfg.concurrency,
        concurrency_max: cfg.concurrency_max,
        effective_cap: effectiveUploadCap(),
        sent: sent,
        failed: failed,
        skipped_dedupe: skipped,
        success_short_circuit: successShortCircuit,
        pagehide_dropped: pagehideDropped,
        soft_abort_on_unload: softAbortOnUnload,
        flush_reason: flushReason,
        hard_anchor_pending: hardPending,
        hard_anchor_flushed: hardFlushed,
        hard_anchor_retries: hardRetries,
        hard_anchor_priority_boost: cfg.hard_anchor_priority_boost || 1000,
        transport_retries: transportRetryCount,
        capture_freezes: captureFreezeCount,
        ack_stored: ackStored,
        ack_duplicate: ackDuplicate,
        ack_merged: ackMerged,
        ack_conflict: ackConflict,
        aimd_ups: aimdUps,
        aimd_downs: aimdDowns,
        registry_n: Object.keys(materialRegistry).length,
        hiding: hiding,
        halted: !!haltState,
        halt_reason: haltState && haltState.reason,
        halt_code: haltState && haltState.code,
        halt_session_id: haltState && haltState.session_id,
        halted_drops: haltedDrops,
      };
    },
    /** Pure helper for tests: sort order under hide with hard boost. */
    sortPreview: function (items, hide) {
      var was = hiding;
      if (hide) hiding = true;
      var copy = (items || []).slice().map(function (it) {
        return {
          batch_id: it.batch_id,
          priority: it.priority || 0,
          effective: effectivePriority(it),
          hard: isHardAnchorBatch(it.batch_id),
        };
      });
      copy.sort(function (a, b) {
        return b.effective - a.effective;
      });
      hiding = was;
      return copy;
    },
    /** Resolve when upload queue is idle (or timeout). Used by multi-tick settle. */
    whenIdle: function (timeoutMs) {
      timeoutMs = timeoutMs == null ? 2000 : timeoutMs;
      var start = Date.now();
      return new Promise(function (resolve) {
        function tick() {
          if (pending.length === 0 && inflight === 0) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW) {}
            resolve(api.stats());
            return;
          }
          if (Date.now() - start >= timeoutMs) {
            try {
              reportSessionUploadSummary(false);
            } catch (eW2) {}
            resolve(api.stats());
            return;
          }
          setTimeout(tick, 16);
        }
        tick();
      });
    },
    /** P2: probe depth class for current session outcome. */
    probeDepthClass: function () {
      try {
        var ok = sessionOutcome.batches_ok || {};
        var hasB0 = !!ok["B0_bootstrap"];
        var hasB10 = !!ok["B10_hw_curves"];
        var hasB8 = !!ok["B8_gateway"];
        var n = Object.keys(ok).length;
        if (hasB8 && !hasB0 && n <= 2) return "gateway_only";
        if (hasB0 && hasB10) return "browser_b0_b10";
        if (hasB0) return "browser_partial";
        if (n) return "browser_lite";
        return "empty";
      } catch (e) {
        return "empty";
      }
    },
    /**
     * Progressive land snapshot (window still open).
     * Used by self_heal / ops to drive missing_probe reduction without waiting for unload.
     */
    completenessSnapshot: function () {
      var ok = sessionOutcome.batches_ok || {};
      var hard = cfg.hard_anchor_batches || [];
      var hardOk = [];
      var hardMiss = [];
      for (var i = 0; i < hard.length; i++) {
        if (ok[hard[i]]) hardOk.push(hard[i]);
        else hardMiss.push(hard[i]);
      }
      var okKeys = Object.keys(ok);
      var ver = "";
      try {
        ver = String(
          global.__GR_PRODUCT_VERSION__ ||
            global.__GR_SERVER_PRODUCT_VERSION__ ||
            global.__GR_FE_PACKS_VERSION__ ||
            ""
        );
      } catch (eV) {}
      return {
        progressive: true,
        product_version: ver,
        probe_depth_class: api.probeDepthClass(),
        hard_total: hard.length,
        hard_ok_n: hardOk.length,
        hard_ok: hardOk,
        hard_missing: hardMiss,
        batches_ok_n: okKeys.length,
        batches_ok: okKeys,
        pending: pending.length,
        inflight: inflight,
        halted: !!haltState,
        hiding: !!hiding,
        sealed_ok: sessionOutcome.sealed_ok || 0,
        network_fail: sessionOutcome.network || 0,
        main_complete: !!(ok["B0_bootstrap"] && ok["B10_hw_curves"]),
      };
    },
    /**
     * Keep draining queue while the tab is open (not only pagehide).
     * Safe no-op when empty / halted.
     */
    pumpWhileOpen: function (why) {
      if (haltState) return api.completenessSnapshot();
      try {
        if (typeof api.flush === "function") {
          api.flush(why || "progressive_open");
        }
      } catch (eP) {}
      try {
        pump();
      } catch (e2) {}
      return api.completenessSnapshot();
    },
  };

  global.GRUploadQueue = api;
})(typeof window !== "undefined" ? window : globalThis);
