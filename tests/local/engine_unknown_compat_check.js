/**
 * Unknown-engine compatibility contract.
 *
 * A UA string alone must not select an engine-specific route. The method
 * matrix must retain compatibility fallbacks until capability evidence
 * converges.
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
const src = fs.readFileSync(path.join(feDir, "probe_method_matrix.js"), "utf8");
const sandbox = {
  navigator: { userAgent: "Mozilla/5.0 Chrome/151.0.0.0 Safari/537.36" },
  console,
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(src, sandbox, { filename: "probe_method_matrix.js" });

const M = sandbox.GRProbeMethodMatrix;
if (!M || M.detectEngine() !== "unknown") {
  throw new Error("UA-only input must remain unknown");
}
const methods = M.methodsFor("B10_hw_curves", "unknown").map((m) => m.profile);
if (!methods.includes("webkit_safe") || !methods.includes("legacy_webgl1")) {
  throw new Error(`unknown engine lost compatibility paths: ${methods.join(",")}`);
}
const b10x = M.methodsFor("B10x_silicon_noderiv", "unknown").map((m) => m.profile);
if (!b10x.includes("compat_surface") || !b10x.includes("legacy_webgl1")) {
  throw new Error(`unknown B10x route lacks fallbacks: ${b10x.join(",")}`);
}
console.log("ENGINE_UNKNOWN_COMPAT_CHECK_PASS", JSON.stringify({ engine: M.detectEngine(), methods, b10x }));
