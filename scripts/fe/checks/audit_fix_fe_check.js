/**
 * Static contract checks for confirmed FE P0s (no browser).
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

// 仓库根解析: 向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
let root = path.dirname(fileURLToPath(import.meta.url));
while (root !== path.parse(root).root && !fs.existsSync(path.join(root, "Cargo.toml"))) root = path.dirname(root);
// 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server(probe/ 在根)
const feDir = fs.existsSync(path.join(root, "02-probe-analysis/probe/fe"))
  ? path.join(root, "02-probe-analysis/probe/fe")
  : path.join(root, "probe/fe");
// FE 源文件位于 02-probe-analysis/probe/fe/（旧线的 fe/ 已不存在）
const uq = fs.readFileSync(path.join(feDir, "upload_queue.js"), "utf8");
const rpa = fs.readFileSync(path.join(feDir, "rpa_monitor.js"), "utf8");

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

assert(/function startUpload\(item, itemHeavy, k\)/.test(uq), "startUpload(item, itemHeavy, k) missing");
assert(/startUpload\(item, itemHeavy, k\)/.test(uq), "pump must call startUpload with bound item");
assert(/durability_state/.test(uq), "applyAck must read durability_state");
assert(/stored_primary_only/.test(uq) && /accepted_pending_durability/.test(uq), "unverified durability states");
assert(/GRRpaMonitor\.ackSegment/.test(uq), "B11 ACK must advance RPA cursor");

assert(/ackedSeq/.test(rpa) && /rpa_seq_start/.test(rpa) && /rpa_seq_end/.test(rpa), "RPA seq fields");
assert(/event_seq: state\.nextSeq\+\+/.test(rpa), "events stamped with event_seq");
assert(/seq > \(state\.ackedSeq \|\| 0\)/.test(rpa), "flush must slice unacked seq");
assert(/ackSegment:\s*function/.test(rpa), "ackSegment export");

const sandbox = { console, setInterval() {}, module: { exports: {} } };
sandbox.self = sandbox;
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(rpa, sandbox, { filename: "rpa_monitor.js" });
const mon = sandbox.GRRpaMonitor;
assert(mon && typeof mon.bindWorker === "function", "bindWorker");
const st = mon.bindWorker({
  source: "worker:test",
  postMessage() {},
});
assert(st && st.ackedSeq === 0, "worker cursor starts at 0");
st.events.push({ event_seq: st.nextSeq++, kind: "worker_tick", type: "worker_tick" });
st.events.push({ event_seq: st.nextSeq++, kind: "worker_tick", type: "worker_tick" });
mon.ackSegment("worker:test", 1, "worker|1-1");
assert(st.ackedSeq === 1, "ackSegment advances worker cursor");
const slice = st.events.filter((e) => Number(e.event_seq) > (st.ackedSeq || 0));
assert(slice.length === 1 && slice[0].event_seq === 2, "incremental slice after ack");

const concat = fs.readFileSync(path.join(feDir, "gr.entry.concat.js"), "utf8");
assert(/function startUpload\(item, itemHeavy, k\)/.test(concat), "concat includes startUpload bind");
assert(/durability_state/.test(concat), "concat includes durability_state");

console.log("FE_AUDIT_FIX_PASS");
