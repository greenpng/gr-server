#!/usr/bin/env node
/**
 * Version-keyed load-path smoke (greenpng 1.0.3+ policy):
 * - pin fixed name only (no-store)
 * - ALL non-pin assets under /dist/v/<fe_version>/g/<asset_gen>/<opaque-hash>.<ext>
 *   → every release/FE rebuild rotates the REAL URL path so browser/CDN caches
 *   can never serve a stale asset or a stale 404 (178 v1.0.2 CF incident)
 * - opaque/content-hash basenames retained (no meaningful product names)
 * - NO `?v=` query busting (stripped on sight)
 * - FE normalizers keep /dist/v/ segments (collapse regexes removed)
 */
"use strict";

const fs = require("fs");
const path = require("path");

// 仓库根解析: 向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
let ROOT = __dirname;
while (ROOT !== path.parse(ROOT).root && !fs.existsSync(path.join(ROOT, "Cargo.toml"))) ROOT = path.dirname(ROOT);
// FE 源文件位于 02-probe-analysis/probe/fe/（旧线的 fe/ 已不存在）
// 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server(probe/ crates/ 在根)
const FE = fs.existsSync(path.join(ROOT, "02-probe-analysis/probe/fe"))
  ? path.join(ROOT, "02-probe-analysis", "probe", "fe")
  : path.join(ROOT, "probe", "fe");
const CRATES = fs.existsSync(path.join(ROOT, "02-probe-analysis/crates"))
  ? path.join(ROOT, "02-probe-analysis", "crates")
  : path.join(ROOT, "crates");

function read(rel) {
  return fs.readFileSync(path.join(FE, rel), "utf8");
}

let failed = 0;
function ok(cond, msg) {
  if (!cond) {
    console.error("FAIL:", msg);
    failed++;
  } else {
    console.log("OK:", msg);
  }
}

const boot = read("gr.boot.js");
const pin = read("gr.js");
const pack = read("pack_loader.js");
const seal = read("gr.seal.js");
const loader = read("gr.loader.js");
const handlers = fs.readFileSync(
  path.join(CRATES, "gr-probe-plane/src/handlers.rs"),
  "utf8"
);
const sealRs = fs.readFileSync(
  path.join(CRATES, "gr-probe-core/src/seal_v2.rs"),
  "utf8"
);

// Server: bootstrap dist_root is version-keyed (fe_version + asset_gen segments)
ok(
  handlers.includes('"{}/dist/v/{}/g/{}"'),
  "bootstrap dist_root is /dist/v/<fe_version>/g/<asset_gen>"
);
ok(
  handlers.includes('let flat_dist = format!("{}/dist", path_prefix'),
  "flat dist root kept as asset_base_flat fallback"
);

// seal grant: content-hash wasm name (path versioning comes from manifest URLs)
ok(
  !sealRs.includes('format!("/g5/dist/v/{fe_epoch}")') &&
    !sealRs.includes('format!("/g5/dist/v/{epoch}")'),
  "seal grant/meta constructs no version path of its own"
);
ok(sealRs.includes("gr_seal_v2.{wasm_hash12}.wasm"), "seal uses content-hash wasm name");

// No FE source collapses /dist/v/ anymore (the anti-leak flatten is retired)
for (const [name, src] of [
  ["gr.boot.js", boot],
  ["gr.js", pin],
  ["pack_loader.js", pack],
]) {
  ok(
    !src.includes("replace(/\\/dist\\/v\\/[^/]+\\/(?:g\\/[^/]+\\/)?/, \"/dist/\")"),
    name + " no /dist/v/ collapse regex"
  );
}

// boot keeps manifest asset_base verbatim (version-keyed)
ok(
  boot.includes("if (ab) {") && boot.includes('return String(ab).replace(/\\/?$/, "/");'),
  "boot distVersionRoot keeps manifest asset_base verbatim"
);

// pin: keeps version-keyed asset_base (no collapse), pin fallback never ?v=
ok(!pin.includes("eCollapse"), "pin applyManifest has no collapse branch");
ok(!/dist\/v\/"\s*\+\s*encodeURIComponent/.test(pin), "pin constructs no /dist/v/ by hand");

// pack_loader keeps versioned URLs (no collapse, no base rewrite)
ok(
  !pack.includes("base = String(base).replace(/\\/dist\\/v\\"),
  "pack_loader relative join does not flatten asset_base"
);

// loader: version-keyed fallback dist root + entry fallback path
ok(
  loader.includes('"/dist/v/" + v + "/"') || loader.includes('"/dist/v/" + fv + "/"'),
  "loader distV builds /dist/v/<V>/"
);
ok(
  loader.includes('fv ? "/dist/v/" + fv + "/" : "/dist/"'),
  "loader entry fallback uses version segment when known"
);
for (const [name, src] of [
  ["gr.loader.js", loader],
  ["gr.boot.js", boot],
]) {
  ok(!src.includes('"?v=" + encodeURIComponent'), name + " no ?v= version query");
}

// seal FE: no hand-built epoch version path (uses manifest URLs)
ok(
  !seal.includes('"/g5/dist/v/" + encodeURIComponent(epoch)'),
  "seal.js no epoch version path"
);

// Runtime unit: withVer keeps version-keyed paths; opaque-only basenames stay
const injectSrc = boot.match(/function injectGenBasename\(url\) \{[\s\S]*?\n  \}/);
const withVerMatch = boot.match(/function withVer\(url\) \{[\s\S]*?\n  \}/);
const distRootMatch = boot.match(/function distVersionRoot\(\) \{[\s\S]*?\n  \}/);
const distFlatMatch = boot.match(/function distFlatRoot\(\) \{[\s\S]*?\n  \}/);
const opaqueLeafMatch = boot.match(
  /function isHashedOrOpaqueLeaf\(name\) \{[\s\S]*?\n  \}/
);
ok(
  !!injectSrc && !!withVerMatch && !!distRootMatch && !!opaqueLeafMatch,
  "extract withVer helpers"
);

const man = {
  fe_version: "1.0.3",
  product_version: "1.0.3",
  asset_gen: "deadbeef0123",
  asset_base: "/g5/dist/v/1.0.3/g/deadbeef0123",
  asset_route: "opaque_content_hash",
  assets: {
    race: "/g5/dist/v/1.0.3/g/deadbeef0123/aabbccddeeff.min.js",
  },
};

const fn = new Function(
  "global",
  `
  function assetGen(){ return global.__GR_ASSET_GEN__ || ""; }
  function scriptVersion(){ return global.__GR_PRODUCT_VERSION__ || ""; }
  ${distFlatMatch ? distFlatMatch[0] : "function distFlatRoot(){ return '/g5/dist/'; }"}
  ${opaqueLeafMatch[0]}
  ${distRootMatch[0]}
  ${injectSrc[0]}
  ${withVerMatch[0]}
  return { withVer: withVer, distVersionRoot: distVersionRoot };
`
);
const global = {
  __GR_MANIFEST__: man,
  __GR_ASSET_GEN__: man.asset_gen,
  __GR_ASSET_BASE__: man.asset_base,
  __GR_PRODUCT_VERSION__: man.product_version,
};
const h = fn(global);

// Relative logical name (non-blocked, e.g. random pack id) → version-keyed
// asset_base + opaque-gen leaf. Meaningful product names stay blocked below.
const joined = h.withVer("R46_spotcheck.js");
ok(
  joined === "/g5/dist/v/1.0.3/g/deadbeef0123/R46_spotcheck.deadbeef0123.js",
  "relative join lands on version-keyed root: " + joined
);

// Opaque-only: meaningful race basename still never invented on the wire
const raceRel = h.withVer("gr.race.min.js");
ok(raceRel === "", "opaque-only: relative race invents no wire name");

// Version-keyed opaque absolute → verbatim passthrough (no flatten!)
const vk = h.withVer(man.assets.race);
ok(
  vk === man.assets.race,
  "version-keyed opaque URL passthrough: " + vk
);

// Legacy FLAT opaque absolute (stale manifest) → kept as-is (no rewrite)
const flatAbs = h.withVer("/g5/dist/aabbccddeeff.min.js");
ok(flatAbs === "/g5/dist/aabbccddeeff.min.js", "flat opaque absolute passthrough");

// ?v= query never used for busting (stripped)
const q = h.withVer("/g5/dist/v/1.0.3/g/deadbeef0123/aabbccddeeff.min.js?v=1.0.2");
ok(q === man.assets.race, "sticky ?v= stripped from version-keyed URL: " + q);

// distVersionRoot returns the manifest base verbatim (version segments intact)
ok(
  h.distVersionRoot() === "/g5/dist/v/1.0.3/g/deadbeef0123/",
  "distVersionRoot keeps version segments: " + h.distVersionRoot()
);

if (failed) {
  console.error("\n" + failed + " check(s) failed");
  process.exit(1);
}
console.log("\nfe_versioned_path_smoke: ALL PASSED");
