/**
 * GR Origin Coordinator — same-origin multi-tab heavy-role intelligence.
 *
 * Scope: ONE browser origin (shop.example.com tabs share this; news.example.com is separate).
 *
 * Roles:
 *   heavy     — may run B10 / hard silicon collect
 *   light     — L1 + RPA + transport only (no heavy re-collect)
 *   transport — flush queue / gap-fill uploads only
 *
 * Mechanisms:
 *   1) navigator.locks (preferred) for atomic heavy lease
 *   2) localStorage lease + heartbeat (fallback + cross-tab visibility)
 *   3) BroadcastChannel for progress / handoff / cool (immediate, best-effort)
 *   4) Durable handoff ticket in localStorage (survives closing tab; storage event
 *      + poll claim) — industry pattern: BC alone drops if no live listener;
 *      GA4/Sentry use beacon for payloads; task *continuation* needs shared durable state
 *      (RxDB leader + IDB; we use LS ticket + Web Locks for lighter footprint).
 *
 * Handoff:
 *   - pagehide/beforeunload: release heavy + write durable unfinished ticket + BC
 *   - survivors (or same-tab multipage nav resume): claim ticket → upgrade heavy → ensureGap
 *   - clear ticket when server has B10 / work satisfied
 *
 * @see reports/FE_PROBE_GLOBAL_INTELLIGENCE_ARCH_V1.md
 * @see reports/MULTI_TAB_HANDOFF_RESEARCH_V1.md
 */
(function (global) {
  "use strict";

  if (global.GROriginCoordinator && global.GROriginCoordinator.__ready) return;

  var CH_NAME = "gr-gpi-v1";
  var LS_LEASE = "_g5_gpi_lease";
  var LS_PROGRESS = "_g5_gpi_prog";
  var LS_HANDOFF = "_g5_gpi_handoff";
  var LS_PEERS = "_g5_gpi_peers";
  var LOCK_NAME = "gr-heavy-b10";
  var HEARTBEAT_MS = 2000;
  var LEASE_TTL_MS = 6000;
  var UPGRADE_AFTER_MS = 10000;
  var PROGRESS_STALE_MS = 12000;
  var HANDOFF_TTL_MS = 120000; // 2 min — claim window after tab close / route change
  var HANDOFF_CLAIM_STALE_MS = 15000; // re-claim if claimer died mid-upgrade
  var PEER_TTL_MS = 10000;

  var tabId = null;
  var role = "light";
  var started = false;
  var bc = null;
  var hbTimer = null;
  var watchTimer = null;
  var lockHeld = false;
  var lockRelease = null;
  var cycleId = "";
  var claimingHandoff = false;
  var stats = {
    becomes_heavy: 0,
    becomes_light: 0,
    handoffs_sent: 0,
    handoffs_recv: 0,
    handoffs_durable_write: 0,
    handoffs_claim: 0,
    upload_handoffs: 0,
    upgrades: 0,
    heartbeats: 0,
    peer_max: 1,
  };

  function now() {
    return Date.now();
  }

  function mintTabId() {
    try {
      var k = "__gr_tab_win_id__";
      var ss = global.sessionStorage;
      if (ss) {
        var ex = ss.getItem(k);
        if (ex) return ex;
        var id = "t" + Math.random().toString(36).slice(2, 10) + now().toString(36).slice(-4);
        ss.setItem(k, id);
        return id;
      }
    } catch (e) {}
    return "t" + Math.random().toString(36).slice(2, 12);
  }

  function debugOn() {
    try {
      return !!(
        global.__GR_DEBUG_GPI__ ||
        global.__GR_DEBUG__ ||
        (global.__GR_BOOT__ && global.__GR_BOOT__.debug_gpi)
      );
    } catch (e) {
      return false;
    }
  }

  function clog(level, msg, detail) {
    if (!debugOn()) return;
    try {
      var fn = (global.console && (global.console[level] || global.console.log)) || null;
      if (!fn) return;
      var line = "[gr:gpi] " + msg;
      if (detail != null) fn.call(global.console, line, detail);
      else fn.call(global.console, line);
    } catch (e) {}
  }

  function ops(code, detail, sev) {
    try {
      if (global.GROps && GROps.report) {
        GROps.report(code, "origin_coord", detail || {}, sev || "info");
      }
    } catch (e) {}
    clog(sev === "warn" || sev === "error" ? "warn" : "info", code, detail || null);
  }

  function timeline(ev, d) {
    try {
      var tl = global.__GR_PROBE_TIMELINE__;
      if (!Array.isArray(tl)) {
        tl = [];
        global.__GR_PROBE_TIMELINE__ = tl;
      }
      tl.push({ t: now(), ev: "gpi_" + ev, d: d || null });
      if (tl.length > 100) tl.splice(0, tl.length - 100);
    } catch (e) {}
    clog("debug", "timeline " + ev, d || null);
  }

  function readJson(key) {
    try {
      var s = global.localStorage && localStorage.getItem(key);
      if (!s) return null;
      return JSON.parse(s);
    } catch (e) {
      return null;
    }
  }

  function writeJson(key, obj) {
    try {
      if (global.localStorage) localStorage.setItem(key, JSON.stringify(obj));
    } catch (e) {}
  }

  function clearKey(key) {
    try {
      if (global.localStorage) localStorage.removeItem(key);
    } catch (e) {}
  }

  function sid() {
    try {
      return String(
        cycleId ||
          global.__GR_SESSION_ID__ ||
          global.__GR_CYCLE_ID__ ||
          (global.GRUploadQueue && GRUploadQueue.cfg && GRUploadQueue.cfg.session_id) ||
          ""
      );
    } catch (e) {
      return cycleId || "";
    }
  }

  function prunePeers(map) {
    var out = map && typeof map === "object" ? map : {};
    var t = now();
    Object.keys(out).forEach(function (k) {
      var p = out[k];
      if (!p || t - Number(p.hb || 0) > PEER_TTL_MS) delete out[k];
    });
    return out;
  }

  function touchPeer() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    all[tabId] = {
      hb: now(),
      role: role,
      cycle: sid(),
      href: (function () {
        try {
          return String(location.pathname || "").slice(0, 80);
        } catch (e) {
          return "";
        }
      })(),
    };
    writeJson(LS_PEERS, all);
    var n = Object.keys(all).length;
    if (n > stats.peer_max) stats.peer_max = n;
    return n;
  }

  function dropPeer() {
    try {
      var all = prunePeers(readJson(LS_PEERS) || {});
      delete all[tabId];
      writeJson(LS_PEERS, all);
    } catch (e) {}
  }

  function peerCount() {
    var all = prunePeers(readJson(LS_PEERS) || {});
    return Object.keys(all).length || (started ? 1 : 0);
  }

  /**
   * Multi-tab brain policy — consumed by self-heal / pack_loader / upload.
   * More tabs ⇒ focus heavy on silicon; light tabs = transport + RPA only.
   */
  function policySnapshot() {
    var n = peerCount();
    var multi = n > 1;
    return {
      peer_count: n,
      multi_tab: multi,
      role: role,
      heavy: role === "heavy",
      // light tabs: transport-first (PostHog/Sentry-style queue drain)
      transport_assist: multi && role !== "heavy",
      // heavy under multi-tab: serialize silicon, less deepen thrash
      serial_heavy: multi && role === "heavy",
      // allow light to skip hard collect (pack_loader already gates)
      defer_heavy_collect: multi && role !== "heavy",
      // slightly faster transport nudge when peers help
      transport_priority: multi ? (role === "heavy" ? 1 : 2) : 1,
    };
  }

  function publishRole() {
    try {
      var pol = policySnapshot();
      global.__GR_GPI_ROLE__ = role;
      global.__GR_GPI_TAB_ID__ = tabId;
      global.__GR_GPI_HEAVY__ = role === "heavy";
      global.__GR_GPI_PEER_COUNT__ = pol.peer_count;
      global.__GR_GPI_POLICY__ = pol;
      global.__GR_GPI_MULTI_TAB__ = !!pol.multi_tab;
    } catch (e) {}
  }

  function setRole(next, why) {
    next = String(next || "light");
    if (next !== "heavy" && next !== "light" && next !== "transport") next = "light";
    if (role === next) {
      publishRole();
      return role;
    }
    var prev = role;
    role = next;
    publishRole();
    if (next === "heavy") stats.becomes_heavy++;
    else stats.becomes_light++;
    timeline("role", { from: prev, to: next, why: why || "" });
    ops(
      "gpi_role",
      { tab: tabId, from: prev, to: next, why: why || "", cycle: sid().slice(0, 28) },
      "info"
    );
    broadcast({ type: "role", tab: tabId, role: role, cycle: sid(), why: why || "" });
    // Nudge self-heal to re-evaluate collect rights immediately.
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
    } catch (eT) {}
    return role;
  }

  function broadcast(msg) {
    try {
      if (bc) bc.postMessage(Object.assign({ t: now(), from: tabId }, msg || {}));
    } catch (e) {}
  }

  function leaseSnapshot() {
    return readJson(LS_LEASE);
  }

  function leaseIsMine(L) {
    return !!(L && L.tab === tabId);
  }

  function leaseAlive(L) {
    if (!L || !L.tab) return false;
    var hb = Number(L.hb || L.at || 0);
    return now() - hb < LEASE_TTL_MS;
  }

  function writeLease(extra) {
    var L = Object.assign(
      {
        tab: tabId,
        at: now(),
        hb: now(),
        cycle: sid(),
      },
      extra || {}
    );
    writeJson(LS_LEASE, L);
    return L;
  }

  function clearMyLease() {
    var L = leaseSnapshot();
    if (leaseIsMine(L)) clearKey(LS_LEASE);
  }

  function writeProgress(patch) {
    var p = Object.assign(
      {
        tab: tabId,
        role: role,
        cycle: sid(),
        at: now(),
        has_b10_local: false,
        phase: "",
      },
      patch || {}
    );
    try {
      if (global.__GR_RECEIVED_BATCHES__) {
        /* noop */
      }
      var st =
        global.GRUploadQueue &&
        GRUploadQueue.materialState &&
        GRUploadQueue.materialState("B10_hw_curves", sid(), "main");
      if (st === "acked") p.has_b10_local = true;
    } catch (e) {}
    try {
      if (global.__GR_CYCLE_PROBE_STATUS__ && global.__GR_CYCLE_PROBE_STATUS__.has_b10) {
        p.has_b10_server = true;
      }
    } catch (e2) {}
    writeJson(LS_PROGRESS, p);
    broadcast({ type: "progress", progress: p });
    return p;
  }

  function serverHasB10() {
    try {
      var cps = global.__GR_CYCLE_PROBE_STATUS__ || {};
      if (cps.has_b10 === true) return true;
      var cov = cps.identity_coverage || {};
      if (cov.has_b10 === true) return true;
    } catch (e) {}
    try {
      var so = global.__GR_SESSION_OUTCOME__ || {};
      if (so.batches_ok && (so.batches_ok.B10_hw_curves || so.batches_ok["mid.curves"])) return true;
    } catch (e2) {}
    return false;
  }

  function canHeavyCollect() {
    if (serverHasB10()) return false;
    if (role === "heavy") return true;
    // No live lease and we need upgrade path
    var L = leaseSnapshot();
    if (!leaseAlive(L)) return role === "heavy";
    return false;
  }

  function isHeavy() {
    return role === "heavy";
  }

  function releaseHeavy(why) {
    if (lockRelease) {
      try {
        lockRelease();
      } catch (e) {}
      lockRelease = null;
    }
    lockHeld = false;
    clearMyLease();
    if (role === "heavy") setRole("light", why || "release");
    broadcast({ type: "lease_free", why: why || "release", tab: tabId });
  }

  function becomeHeavy(why) {
    writeLease({ why: why || "acquire" });
    lockHeld = true;
    setRole("heavy", why || "acquire");
    writeProgress({ role: "heavy", phase: "heavy_start" });
    stats.becomes_heavy++;
  }

  /**
   * Try to acquire heavy role. Async; resolves with boolean.
   */
  function tryAcquireHeavy(why) {
    return new Promise(function (resolve) {
      if (serverHasB10()) {
        setRole("light", "server_has_b10");
        resolve(false);
        return;
      }
      var L0 = leaseSnapshot();
      if (leaseAlive(L0) && !leaseIsMine(L0)) {
        setRole("light", "other_leader");
        resolve(false);
        return;
      }
      // Prefer Web Locks — if busy, stay light (do NOT fall through to storage race).
      try {
        if (global.navigator && navigator.locks && typeof navigator.locks.request === "function") {
          var settled = false;
          function finish(ok, whyF) {
            if (settled) return;
            settled = true;
            if (!ok) setRole("light", whyF || "lock_busy");
            resolve(!!ok);
          }
          navigator.locks
            .request(LOCK_NAME, { ifAvailable: true }, function (lock) {
              if (!lock) {
                finish(false, "lock_busy");
                return;
              }
              // Re-check storage lease after lock grant (another tab may have storage-only heavy).
              var L1 = leaseSnapshot();
              if (leaseAlive(L1) && !leaseIsMine(L1)) {
                finish(false, "storage_peer");
                return;
              }
              lockHeld = true;
              var released = false;
              lockRelease = function () {
                if (released) return;
                released = true;
              };
              becomeHeavy(why || "web_lock");
              finish(true, "web_lock");
              return new Promise(function (holdDone) {
                var iv = setInterval(function () {
                  if (!lockHeld || role !== "heavy") {
                    clearInterval(iv);
                    holdDone();
                  }
                }, 500);
              });
            })
            .catch(function () {
              // Locks API error → storage fallback only
              if (!settled) storageAcquire(why, resolve);
            });
          return;
        }
      } catch (eL) {}
      storageAcquire(why, resolve);
    });
  }

  function storageAcquire(why, resolve) {
    // Optimistic atomic-ish: re-read, write only if free/stale/mine
    var L = leaseSnapshot();
    if (leaseAlive(L) && !leaseIsMine(L)) {
      setRole("light", "storage_busy");
      resolve(false);
      return;
    }
    // Claim with random token to detect clobber
    var token = tabId + ":" + Math.random().toString(36).slice(2, 8);
    writeLease({ why: why || "storage_lease", token: token });
    // Verify we still own after write (last-writer check)
    var L2 = leaseSnapshot();
    if (!L2 || L2.tab !== tabId) {
      setRole("light", "storage_lost_race");
      resolve(false);
      return;
    }
    becomeHeavy(why || "storage_lease");
    resolve(true);
  }

  function heartbeat() {
    stats.heartbeats++;
    touchPeer();
    publishRole();
    // Nudge self-heal adaptive when multi-tab policy changes
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.applyAdaptivePolicy) {
        GRProbeSelfHeal.applyAdaptivePolicy(true);
      }
    } catch (eP) {}
    if (role === "heavy") {
      writeLease({ hb: now(), cycle: sid() });
      writeProgress({ role: "heavy", phase: "heartbeat", peers: peerCount() });
      if (serverHasB10()) {
        var Hu = readHandoff();
        if (!Hu || !Hu.upload_pending || !Hu.upload_pending.length) clearHandoff("heavy_server_b10");
      }
    } else {
      writeProgress({ role: role, phase: "follower", peers: peerCount() });
      maybeUpgrade();
      processHandoff("heartbeat");
      // Light multi-tab: actively assist transport (queue pump)
      try {
        var pol = policySnapshot();
        if (pol.transport_assist && global.GRUploadQueue && GRUploadQueue.flush) {
          // soft background flush — not unloading
          GRUploadQueue.flush("gpi_transport_assist");
        }
      } catch (eT) {}
    }
  }

  function maybeUpgrade() {
    if (role === "heavy") return;
    if (serverHasB10()) return;
    var L = leaseSnapshot();
    var prog = readJson(LS_PROGRESS);
    // Leader dead or progress stale → upgrade
    var leaderDead = !leaseAlive(L);
    var progressStale =
      !prog || !prog.at || now() - Number(prog.at) > PROGRESS_STALE_MS || prog.tab !== (L && L.tab);
    var waited = started ? now() - (global.__GR_GPI_START_MS__ || now()) : 0;
    if (leaderDead || (progressStale && waited > UPGRADE_AFTER_MS)) {
      tryAcquireHeavy(leaderDead ? "upgrade_lease_dead" : "upgrade_stale_progress").then(function (ok) {
        if (ok) {
          stats.upgrades++;
          timeline("upgrade", { why: leaderDead ? "lease_dead" : "stale" });
          ops("gpi_upgrade", { tab: tabId, why: leaderDead ? "lease_dead" : "stale" }, "warn");
        }
      });
    }
  }

  function onMessage(ev) {
    var m = (ev && ev.data) || {};
    if (!m || m.from === tabId) return;
    if (m.type === "progress" && m.progress) {
      try {
        // Cache peer progress for diagnostics
        global.__GR_GPI_PEER__ = m.progress;
      } catch (e) {}
      if (m.progress.has_b10_local || m.progress.has_b10_server) {
        if (role === "heavy" && !serverHasB10()) {
          /* keep heavy until server confirms — avoid flip-flop */
        } else if (role === "heavy" && serverHasB10()) {
          releaseHeavy("peer_or_server_b10");
        }
      }
    }
    if (m.type === "handoff") {
      stats.handoffs_recv++;
      timeline("handoff_recv", m);
      ops(
        "gpi_handoff_recv",
        {
          from: m.from,
          unfinished: m.unfinished || [],
          cycle: m.cycle,
          durable: !!m.durable,
        },
        "warn"
      );
      // Mirror into durable store if peer only BC'd (older path / race)
      if (Array.isArray(m.unfinished) && m.unfinished.length && !serverHasB10()) {
        var cur = readHandoff();
        if (!handoffIsActionable(cur)) {
          writeJson(LS_HANDOFF, {
            v: 1,
            from: m.from,
            cycle: m.cycle || sid(),
            unfinished: m.unfinished.slice(),
            at: now(),
            why: "bc_mirror",
            claimed_by: null,
            claimed_at: 0,
          });
        }
      }
      processHandoff("bc");
    }
    if (m.type === "cool" || m.type === "has_b10") {
      if (serverHasB10()) clearHandoff("peer_has_b10");
      if (role === "heavy") {
        // Demote after peer cool if server has b10
        if (serverHasB10()) releaseHeavy("peer_cool");
      }
    }
    if (m.type === "lease_free") {
      maybeUpgrade();
      processHandoff("lease_free");
    }
  }

  function collectUnfinished() {
    var out = [];
    try {
      if (global.GRProbeSelfHeal && GRProbeSelfHeal.snapshot) {
        var snap = GRProbeSelfHeal.snapshot();
        (snap.gaps || []).forEach(function (g) {
          if (g && g.desired && !g.satisfied && (g.batch_id === "B10_hw_curves" || g.batch_id === "mid.curves")) {
            out.push(g.batch_id);
          }
        });
      }
    } catch (e) {}
    // Also inspect pack health — heavy mid-flight may not have gap entry yet
    try {
      var packs = global.__GR_PACK_HEALTH__ || {};
      var b10h = packs.B10_hw_curves || packs["mid.curves"];
      if (b10h && (b10h.run === "running" || b10h.run === "queued" || b10h.state === "running")) {
        if (out.indexOf("B10_hw_curves") < 0) out.push("B10_hw_curves");
      }
    } catch (e2) {}
    if (!out.length && !serverHasB10() && role === "heavy") out.push("B10_hw_curves");
    // de-dup
    var seen = {};
    return out.filter(function (x) {
      if (seen[x]) return false;
      seen[x] = 1;
      return true;
    });
  }

  /**
   * Pending upload batch ids (not full payloads — sealed bodies too large for LS).
   * Survivor re-collects if server missing; otherwise transport nudge only.
   * Mirrors PostHog retry-queue persistence of *request identity*, not always body.
   */
  function collectUploadPending() {
    var out = [];
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.stats === "function") {
        var st = q.stats() || {};
        var pend = st.pending_batches || st.pending_ids || st.batches_pending || null;
        if (Array.isArray(pend)) {
          pend.forEach(function (b) {
            if (b && out.indexOf(String(b)) < 0) out.push(String(b));
          });
        }
      }
      // material state scan for hard anchors still not acked
      if (q && typeof q.materialState === "function") {
        ["B10_hw_curves", "mid.curves", "B0_bootstrap", "B2_hardware", "B3_system"].forEach(function (bid) {
          try {
            var ms = q.materialState(bid, sid(), "main");
            if (ms && ms !== "acked" && ms !== "absent" && out.indexOf(bid) < 0) {
              if (ms === "queued" || ms === "uploading" || ms === "retry_wait" || ms === "collected" || ms === "new") {
                out.push(bid);
              }
            }
          } catch (eM) {}
        });
      }
    } catch (e) {}
    return out.slice(0, 24);
  }

  function readHandoff() {
    return readJson(LS_HANDOFF);
  }

  function handoffIsActionable(H) {
    if (!H) return false;
    var at = Number(H.at || 0);
    if (!at || now() - at > HANDOFF_TTL_MS) return false;
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    var needUpload = Array.isArray(H.upload_pending) && H.upload_pending.length > 0;
    if (!needCollect && !needUpload) return false;
    // Peer already claimed and still looks alive → leave it (collect path)
    if (needCollect && H.claimed_by && H.claimed_by !== tabId) {
      var cAt = Number(H.claimed_at || 0);
      var claimFresh = cAt && now() - cAt < HANDOFF_CLAIM_STALE_MS;
      var L = leaseSnapshot();
      var claimerHoldsLease = leaseAlive(L) && L.tab === H.claimed_by;
      if (claimerHoldsLease) return false;
      if (claimFresh && leaseAlive(L) && L.tab !== tabId) return false;
    }
    return true;
  }

  /**
   * Durable ticket: localStorage survives the dying tab.
   * BC is best-effort notify for already-open survivors.
   * Same-tab multipage navigation also pagehides — ticket lets the next document resume.
   */
  function writeDurableHandoff(unfinished, why, uploadPending) {
    uploadPending = uploadPending || [];
    var needCollect = unfinished && unfinished.length && !serverHasB10();
    var needUpload = uploadPending && uploadPending.length;
    if (!needCollect && !needUpload) return null;
    var ticket = {
      v: 2,
      from: tabId,
      cycle: sid(),
      unfinished: needCollect ? unfinished.slice() : [],
      upload_pending: needUpload ? uploadPending.slice(0, 24) : [],
      at: now(),
      why: why || "pagehide",
      href: (function () {
        try {
          return String(location.href || "").slice(0, 160);
        } catch (e) {
          return "";
        }
      })(),
      role_was: role,
      claimed_by: null,
      claimed_at: 0,
    };
    // Prefer keep fresher ticket if same cycle still open
    var prev = readHandoff();
    if (prev && (handoffIsActionable(prev) || (prev.upload_pending && prev.upload_pending.length)) && prev.cycle === ticket.cycle) {
      var merged = (prev.unfinished || []).slice();
      (ticket.unfinished || []).forEach(function (u) {
        if (merged.indexOf(u) < 0) merged.push(u);
      });
      ticket.unfinished = merged;
      var um = (prev.upload_pending || []).slice();
      (ticket.upload_pending || []).forEach(function (u) {
        if (um.indexOf(u) < 0) um.push(u);
      });
      ticket.upload_pending = um.slice(0, 24);
      if (prev.claimed_by && prev.claimed_by !== tabId) {
        ticket.claimed_by = prev.claimed_by;
        ticket.claimed_at = prev.claimed_at;
      }
    }
    writeJson(LS_HANDOFF, ticket);
    stats.handoffs_durable_write++;
    if (ticket.upload_pending && ticket.upload_pending.length) stats.upload_handoffs++;
    timeline("handoff_durable", {
      unfinished: ticket.unfinished,
      upload_pending: ticket.upload_pending,
      why: why || "pagehide",
    });
    ops(
      "gpi_handoff_durable",
      {
        unfinished: ticket.unfinished,
        upload_pending: ticket.upload_pending,
        tab: tabId,
        why: why || "pagehide",
      },
      "warn"
    );
    return ticket;
  }

  function clearHandoff(why) {
    var H = readHandoff();
    if (!H) return;
    // Only clear if we own claim or ticket is ours / satisfied
    if (H.claimed_by && H.claimed_by !== tabId && handoffIsActionable(H)) return;
    clearKey(LS_HANDOFF);
    timeline("handoff_clear", { why: why || "done" });
    clog("info", "handoff_clear", { why: why || "done" });
  }

  function kickUploadAssist(uploadPending, reason) {
    if (!uploadPending || !uploadPending.length) return;
    try {
      var q = global.GRUploadQueue;
      if (q && typeof q.nudgeTransport === "function") {
        uploadPending.forEach(function (bid) {
          try {
            q.nudgeTransport(String(bid), sid());
          } catch (eN) {}
        });
      }
      if (q && typeof q.flush === "function") {
        q.flush("handoff_assist");
      }
    } catch (eF) {}
    try {
      if (global.GRProbeSelfHeal) {
        uploadPending.forEach(function (bid) {
          if (!bid) return;
          // Server missing → recollect; else transport loop will settle.
          if (GRProbeSelfHeal.ensureGap) {
            GRProbeSelfHeal.ensureGap(String(bid), "main", {
              desired: true,
              force_recollect: !serverHasB10() && (bid === "B10_hw_curves" || bid === "mid.curves"),
              reason: reason || "upload_handoff",
            });
          }
        });
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
  }

  function kickHeavyWork(reason) {
    // Clear stale light-defer markers so pack_loader will re-enter B10 after upgrade.
    try {
      var ph = global.__GR_PACK_HEALTH__;
      if (ph && typeof ph === "object") {
        ["B10_hw_curves", "mid.curves"].forEach(function (k) {
          if (ph[k] && ph[k].run === "deferred_gpi_light") {
            ph[k].run = "handoff_pending";
            ph[k].handoff_reason = reason || "handoff";
          }
        });
      }
    } catch (ePh) {}
    try {
      if (global.GRProbeSelfHeal) {
        if (GRProbeSelfHeal.ensureGap) {
          GRProbeSelfHeal.ensureGap("B10_hw_curves", "main", {
            desired: true,
            force_recollect: true,
            reason: reason || "handoff",
          });
        }
        if (GRProbeSelfHeal.tick) GRProbeSelfHeal.tick();
      }
    } catch (eH) {}
    try {
      var H = readHandoff();
      if (H && H.upload_pending) kickUploadAssist(H.upload_pending, reason);
    } catch (eU) {}
  }

  /**
   * Claim durable handoff and upgrade to heavy if needed.
   * Idempotent; safe to call from heartbeat / storage / BC / start.
   */
  function processHandoff(source) {
    if (claimingHandoff) return;
    var H = readHandoff();
    if (!handoffIsActionable(H)) {
      // stale cleanup
      if (H) {
        var empty =
          (!H.unfinished || !H.unfinished.length) &&
          (!H.upload_pending || !H.upload_pending.length);
        var expired = now() - Number(H.at || 0) > HANDOFF_TTL_MS;
        if (empty || expired) {
          if (!H.claimed_by || H.claimed_by === tabId) clearKey(LS_HANDOFF);
        }
      }
      if (serverHasB10() && H && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("server_has_b10");
      }
      return;
    }
    // Upload assist: any tab (light or heavy) can help drain / recollect missing
    if (H.upload_pending && H.upload_pending.length) {
      kickUploadAssist(H.upload_pending, "upload_handoff_" + (source || "poll"));
    }
    var needCollect =
      !serverHasB10() &&
      Array.isArray(H.unfinished) &&
      H.unfinished.some(function (x) {
        return x === "B10_hw_curves" || x === "mid.curves";
      });
    if (!needCollect) {
      // only upload path — no heavy upgrade required
      if (serverHasB10() && (!H.upload_pending || !H.upload_pending.length)) {
        clearHandoff("upload_done");
      }
      return;
    }
    // If we are already heavy, just kick work and mark claim
    if (role === "heavy") {
      H.claimed_by = tabId;
      H.claimed_at = now();
      writeJson(LS_HANDOFF, H);
      kickHeavyWork("handoff_already_heavy");
      return;
    }
    claimingHandoff = true;
    // Optimistic claim stamp (helps multi-survivor race)
    H.claimed_by = tabId;
    H.claimed_at = now();
    writeJson(LS_HANDOFF, H);
    tryAcquireHeavy(source === "start" ? "handoff_resume" : "handoff").then(function (ok) {
      claimingHandoff = false;
      if (!ok) {
        // Lost race — leave ticket for winner; clear our claim if still us
        var H2 = readHandoff();
        if (H2 && H2.claimed_by === tabId) {
          H2.claimed_by = null;
          H2.claimed_at = 0;
          writeJson(LS_HANDOFF, H2);
        }
        // Still help upload as light
        kickUploadAssist((H && H.upload_pending) || [], "light_upload_assist");
        return;
      }
      stats.handoffs_claim++;
      stats.handoffs_recv++;
      timeline("handoff_claim", { source: source || "poll", unfinished: H.unfinished });
      ops(
        "gpi_handoff_claim",
        { tab: tabId, source: source || "poll", unfinished: H.unfinished },
        "warn"
      );
      kickHeavyWork("handoff_claim");
    });
  }

  function onPageHide() {
    // Best-effort flush uploads first (GA4/Sentry/PostHog pattern)
    try {
      if (global.GRUploadQueue && GRUploadQueue.flush) {
        GRUploadQueue.flush("pagehide");
      }
    } catch (eFl) {}
    var unfinished = collectUnfinished();
    var uploadPending = collectUploadPending();
    if (unfinished.length || uploadPending.length) {
      stats.handoffs_sent++;
      writeDurableHandoff(
        unfinished,
        role === "heavy" ? "pagehide_heavy" : "pagehide",
        uploadPending
      );
      broadcast({
        type: "handoff",
        unfinished: unfinished,
        upload_pending: uploadPending,
        cycle: sid(),
        role: role,
        durable: true,
      });
      timeline("handoff_send", {
        unfinished: unfinished,
        upload_pending: uploadPending,
        durable: true,
      });
      ops(
        "gpi_handoff_send",
        {
          unfinished: unfinished,
          upload_pending: uploadPending,
          tab: tabId,
          durable: true,
        },
        "warn"
      );
    }
    dropPeer();
    if (role === "heavy") releaseHeavy("pagehide");
  }

  function start(opts) {
    opts = opts || {};
    if (started) return api;
    started = true;
    tabId = mintTabId();
    try {
      global.__GR_GPI_START_MS__ = now();
    } catch (e) {}
    if (opts.cycle_id) cycleId = String(opts.cycle_id);
    try {
      cycleId = cycleId || sid();
    } catch (e2) {}

    try {
      if (typeof BroadcastChannel !== "undefined") {
        bc = new BroadcastChannel(CH_NAME);
        bc.onmessage = onMessage;
      }
    } catch (eBc) {
      bc = null;
    }

    try {
      global.addEventListener("pagehide", onPageHide);
      global.addEventListener("beforeunload", onPageHide);
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "visible") {
            heartbeat();
            maybeUpgrade();
          }
        });
      }
      global.addEventListener("storage", function (ev) {
        if (!ev) return;
        if (ev.key === LS_LEASE || ev.key === LS_PROGRESS) {
          maybeUpgrade();
        }
        if (ev.key === LS_HANDOFF) {
          processHandoff("storage");
        }
      });
    } catch (eH) {}

    // Initial role acquisition; then process any durable handoff left by a closed tab
    // or by same-tab multipage navigation (pagehide wrote ticket before unload).
    tryAcquireHeavy(opts.why || "start").then(function (ok) {
      if (!ok) setRole("light", "start_follower");
      publishRole();
      // Slight delay so self-heal/pack_loader are ready for ensureGap
      setTimeout(function () {
        processHandoff("start");
      }, 80);
    });

    hbTimer = setInterval(heartbeat, HEARTBEAT_MS);
    watchTimer = setInterval(function () {
      maybeUpgrade();
      processHandoff("watch");
      if (serverHasB10()) {
        clearHandoff("watch_server_b10");
        if (role === "heavy") {
          // Keep heavy briefly for deepen; demote after coverage stable
          writeProgress({ has_b10_server: true, phase: "b10_done" });
          broadcast({ type: "has_b10", cycle: sid() });
        }
      }
    }, 3000);

    timeline("start", { tab: tabId });
    ops("gpi_start", { tab: tabId }, "info");
    return api;
  }

  function stop(why) {
    started = false;
    if (hbTimer) {
      try {
        clearInterval(hbTimer);
      } catch (e) {}
      hbTimer = null;
    }
    if (watchTimer) {
      try {
        clearInterval(watchTimer);
      } catch (e2) {}
      watchTimer = null;
    }
    onPageHide();
    try {
      if (bc) bc.close();
    } catch (e3) {}
    bc = null;
    ops("gpi_stop", { why: why || "stop", tab: tabId }, "info");
  }

  function snapshot() {
    return {
      started: started,
      tab: tabId,
      role: role,
      heavy: role === "heavy",
      can_heavy: canHeavyCollect(),
      lock_held: lockHeld,
      lease: leaseSnapshot(),
      progress: readJson(LS_PROGRESS),
      handoff: readHandoff(),
      peers: peerCount(),
      policy: policySnapshot(),
      stats: Object.assign({}, stats),
      cycle: sid(),
      server_has_b10: serverHasB10(),
    };
  }

  var api = {
    __ready: true,
    start: start,
    stop: stop,
    tryAcquireHeavy: tryAcquireHeavy,
    releaseHeavy: releaseHeavy,
    canHeavyCollect: canHeavyCollect,
    isHeavy: isHeavy,
    getRole: function () {
      return role;
    },
    getTabId: function () {
      return tabId;
    },
    peerCount: peerCount,
    policy: policySnapshot,
    snapshot: snapshot,
    writeProgress: writeProgress,
    serverHasB10: serverHasB10,
    processHandoff: processHandoff,
    clearHandoff: clearHandoff,
    readHandoff: readHandoff,
  };

  global.GROriginCoordinator = api;
})(typeof window !== "undefined" ? window : globalThis);
