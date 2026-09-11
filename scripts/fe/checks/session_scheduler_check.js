/**
 * SessionScheduler contract checks.
 *
 * Upload transport is decoupled from collection: pending/inflight sealed
 * uploads must not prevent the next scheduler kick. Callers that truly need
 * upload quiescence can opt into wait_for_upload.
 */
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

// 仓库根解析: 向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
let root = path.dirname(fileURLToPath(import.meta.url));
while (root !== path.parse(root).root && !fs.existsSync(path.join(root, "Cargo.toml"))) root = path.dirname(root);
// 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server(probe/ 在根)
const feDir = fs.existsSync(path.join(root, "02-probe-analysis/probe/fe"))
  ? path.join(root, "02-probe-analysis/probe/fe")
  : path.join(root, "probe/fe");
const src = fs.readFileSync(path.join(feDir, "session_scheduler.js"), "utf8");

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

let kicks = 0;
const sandbox = {
  setInterval() {},
  clearInterval() {},
  GRUploadQueue: {
    stats() {
      return { pending: 2, inflight: 1 };
    },
  },
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(src, sandbox, { filename: "session_scheduler.js" });

const S = sandbox.GRSessionScheduler;
S.bind({ kick() { kicks += 1; }, debounce_ms: 0 });
const next = S.submit("upload_overlap");
assert(next.ok === true && kicks === 1, "upload-only activity must not block collection");
assert(S.isBusy() === false, "upload-only activity must not report scheduler busy");
assert(S.uploadIsBusy() === true, "upload activity remains observable");
assert(S.snapshot().upload_busy === true, "snapshot exposes upload_busy");

const strict = S.submit("strict_transport", { wait_for_upload: true });
assert(strict.ok === false && strict.reason === "busy", "explicit upload wait must defer");
assert(S.snapshot().stats.deferred_busy === 1, "explicit upload wait is counted");

sandbox.__GR_MULTI_TICK_ACTIVE__ = true;
const active = S.submit("active_cycle");
assert(active.ok === false && active.reason === "busy", "active collection must defer");
assert(S.isBusy() === true, "active collection reports scheduler busy");

// iss/audit PRB-02: a throwing kick must surface failure honestly — the old
// empty catch returned { ok: true } and the probe cycle silently died.
sandbox.__GR_MULTI_TICK_ACTIVE__ = false;
const opsReports = [];
sandbox.GROps = {
  report(code, stage, detail, level) {
    opsReports.push({ code, stage, level, detail });
  },
};
S.bind({ kick() { throw new Error("mock kick failure"); }, debounce_ms: 0 });
const failed = S.submit("kick_throws", { force: true });
assert(failed.ok === false, "throwing kick must return ok:false (was ok:true via empty catch)");
assert(/mock kick failure/.test(String(failed.error || "")), "failure detail is propagated");
assert((S.snapshot().stats.kick_errors || 0) >= 1, "kick errors are counted in stats");
assert(
  opsReports.some((r) => r.code === "scheduler_kick_fail" && r.level === "error"),
  "kick failure is reported through GROps at error level"
);

console.log("SESSION_SCHEDULER_CHECK_PASS", JSON.stringify({ kicks, snapshot: S.snapshot(), opsReports }));
