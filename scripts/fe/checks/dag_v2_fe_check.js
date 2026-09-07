/**
 * FE probe_dag_v2 scheduler checks (no browser).
 * Loads pack_loader.js in a VM sandbox, feeds a route_plan whose packs carry
 * dag_v2 metadata, and asserts:
 *   1. engine profile gate skips e.g. B10x_webkit_gl_noise on a Blink claim,
 *      with an observable reason in __GR_DAG_V2_CONSUMED__.skipped;
 *   2. depends_on defers once (dep_wait recorded), then still runs (soft dep);
 *   3. deadline bookkeeping is published per pack with engine modifier;
 *   4. dagUploadStamp() carries version + engine + skipped count for B0/seal.
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
// FE 源文件位于 02-probe-analysis/probe/fe/（旧线的 fe/ 已不存在）
const src = fs.readFileSync(path.join(feDir, "pack_loader.js"), "utf8");

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

const done = [];
const sandbox = {
  console,
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
  addEventListener() {},
  dispatchEvent() {},
  CustomEvent: function (type, init) {
    return { type, detail: (init && init.detail) || {} };
  },
  location: { href: "https://local.test/", origin: "https://local.test", pathname: "/" },
  document: {
    createElement() {
      return { set src(v) {}, async: false, onload: null, onerror: null };
    },
    head: { appendChild() {} },
    documentElement: { appendChild() {} },
  },
  navigator: {
    userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    userAgentData: { brands: [{ brand: "Chromium", version: "120" }] },
  },
  // This fixture intentionally represents a converged Blink capability
  // profile. UA alone is not sufficient for engine selection.
  chrome: { runtime: {} },
  screen: { width: 1920, height: 1080 },
  __GR_RESOURCE_BUS__: null,
  GRCollectors: {
    __h: { detectEngineFamily() { return "blink"; } },
    packsFromRoutePlan(routePlan) {
      return (routePlan.packs || []).map((p) => ({
        id: p.pack_id || p.id,
        pack_id: p.pack_id || p.id,
        batch_id: p.batch_id || p.pack_id || p.id,
        priority: p.priority || 100,
        schedule: p.schedule || "static",
        dag_v2: p.dag_v2,
        run() {
          done.push(p.pack_id || p.id);
          return Promise.resolve({ ok: true, batch_id: p.batch_id || p.pack_id || p.id });
        },
      }));
    },
  },
  GRUploadQueue: {
    isHalted() {
      return false;
    },
    alreadySent() {
      return false;
    },
  },
};

sandbox.self = sandbox;
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(src, sandbox, { filename: "pack_loader.js" });
const PL = sandbox.GRPackLoader;
assert(PL && typeof PL.applyRoutePlan === "function", "GRPackLoader.applyRoutePlan");
assert(typeof PL.dagSnapshot === "function", "dagSnapshot export");
assert(typeof PL.dagUploadStamp === "function", "dagUploadStamp export");

const DAG = {
  pack_id: "B10_hw_curves",
  batch_id: "B10_hw_curves",
  planes: ["S1", "S2"],
  depends_on: ["B0_bootstrap"],
  resource: ["gpu"],
  deadline_ms: 90000,
  engine_profiles: ["blink", "gecko", "webkit", "android"],
  cost_class: "heavy",
  source_kind: "fe",
  commercial_roles: ["silicon_candidate"],
  diagnostic_roles: ["coverage"],
  fallbacks: ["B10x_softgl_hedge", "B10x_legacy_webgl1", "honest_skip"],
  missing_state: "degraded",
  replay_binding: "challenge_seed",
};

const plan = {
  version: "v5.2-directions",
  dag_version: 2,
  session_id: "s_dag_test",
  plan_version: 7,
  plan_epoch: 7,
  packs: [
    { pack_id: "B10_hw_curves", batch_id: "B10_hw_curves", priority: 1000, schedule: "static", dag_v2: DAG },
    {
      pack_id: "B10x_webkit_gl_noise",
      batch_id: "B10x_webkit_gl_noise",
      priority: 800,
      schedule: "dynamic",
      dag_v2: {
        pack_id: "B10x_webkit_gl_noise",
        planes: ["S1", "S2"],
        depends_on: [],
        resource: ["gpu"],
        deadline_ms: 30000,
        engine_profiles: ["webkit"],
        cost_class: "heavy",
        fallbacks: ["honest_skip"],
      },
    },
    { pack_id: "B3_system", batch_id: "B3_system", priority: 300, schedule: "static", dag_v2: { depends_on: [], deadline_ms: 8000, engine_profiles: ["blink", "gecko", "webkit", "ios", "android", "webview", "vm"], cost_class: "light" } },
    {
      pack_id: "B18_webgpu",
      batch_id: "B18_webgpu",
      priority: 560,
      schedule: "dynamic",
      dag_v2: {
        pack_id: "B18_webgpu",
        planes: ["S1"],
        depends_on: [],
        resource: ["gpu"],
        deadline_ms: 45000,
        engine_profiles: ["blink", "gecko", "webkit", "android"],
        cost_class: "heavy",
        gate: "research",
        dual_kpi_gate: "research",
        observation_bound: "single-visit WebGPU adapter+limits; dual powerPreference",
        independent_failure_mode: ["webgpu_unavailable", "adapter_null", "validation_error"],
        cost_budget: { upload_bytes_est: 24000, rows_per_session: 400, analysis_cost_class: "heavy" },
      },
    },
    { pack_id: "B4_mobile", batch_id: "B4_mobile", priority: 200, schedule: "dynamic", dag_v2: { depends_on: [], deadline_ms: 8000, engine_profiles: ["blink", "gecko", "webkit", "ios", "android", "webview", "vm"], cost_class: "light", gate: "default" } },
  ],
  parallel_groups: [],
  stop_probe: false,
  static_pack_ids: ["B10_hw_curves", "B3_system"],
  dynamic_pack_ids: ["B10x_webkit_gl_noise", "B18_webgpu", "B4_mobile"],
  notes: "",
};

const ctx = {
  session_id: "s_dag_test",
  queue: sandbox.GRUploadQueue,
  apiBase: "/g5",
  site_id: "t",
  source: "main",
};

const applied = await PL.applyRoutePlan(plan, ctx);
const snap = PL.dagSnapshot();

// 1) engine gate on Blink claim
assert(
  snap.skipped["B10x_webkit_gl_noise"] === "engine_profile_excluded:blink",
  "webkit lane must be skipped on blink claim: " + JSON.stringify(snap.skipped)
);
assert(applied.kicked.indexOf("B10_hw_curves") >= 0, "B10 must run on blink");
assert(applied.kicked.indexOf("B3_system") >= 0, "light pack must run");
assert(applied.kicked.indexOf("B4_mobile") >= 0, "default-gated dynamic pack must run");
assert(applied.skipped.indexOf("B10x_webkit_gl_noise") >= 0, "skipped list must include webkit lane");

// 1b) research four-piece gate: B18 stays out of the default schedule
//     (gate_research_hold), a default-gated pack with the same profile runs.
assert(
  snap.skipped["B18_webgpu"] === "gate_research_hold",
  "research-gated B18 must be held: " + JSON.stringify(snap.skipped)
);
assert(applied.skipped.indexOf("B18_webgpu") >= 0, "skipped list must include research B18");

// 2) depends_on: B0_bootstrap missing from plan → dep_wait recorded, soft-run after defer
assert(snap.dep_wait["B10_hw_curves"], "dep_wait must be recorded: " + JSON.stringify(snap.dep_wait));
assert(done.indexOf("B10_hw_curves") >= 0, "B10 must still run (soft dep after one defer)");

// 3) deadlines published with engine modifier (blink → no modifier)
assert(snap.deadlines["B10_hw_curves"], "deadline bookkeeping for B10");
assert(snap.deadlines["B10_hw_curves"].deadline_ms === 90000, "blink deadline unmodified");
assert(snap.deadlines["B10_hw_curves"].cost_class === "heavy", "cost_class in deadline record");

// 4) upload stamp for B0/seal binding
const stamp = PL.dagUploadStamp();
assert(stamp.dag_v2_version === 2, "stamp version");
assert(stamp.dag_v2_engine === "blink", "stamp engine: " + stamp.dag_v2_engine);
assert(stamp.dag_v2_skipped_n >= 1, "stamp skipped count");

// UA is recorded as a claim, never used to override feature evidence.
assert(snap.ua_claim.indexOf("Chrome/120") >= 0, "ua_claim recorded separately");

// reset clears observability (new cycle)
PL.dagReset();
assert(Object.keys(PL.dagSnapshot().skipped || {}).length === 0, "reset clears skipped");

console.log("DAG_V2_FE_CHECK_PASS", JSON.stringify({ kicked: applied.kicked, skipped: applied.skipped, engine: snap.engine }));
