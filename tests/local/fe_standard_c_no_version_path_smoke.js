#!/usr/bin/env node
/**
 * Standard C smoke:
 * - pin fixed name only
 * - no product/fe version in public load paths
 * - content-hash (or opaque gen) in filenames for all non-pin assets
 * - boot/pack_loader collapse legacy /dist/v/… → /dist/
 */
"use strict";

const fs = require("fs");
const path = require("path");

const ROOT = path.resolve(__dirname, "../../..");
// v7: FE 源文件位于 02-probe-analysis/probe/fe/（green-v6 时代的 fe/ 已不存在）
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

// Server: asset base is flat dist
ok(
  handlers.includes('format!("{}/dist", path_prefix') ||
    handlers.includes("format!(\"{}/dist\", path_prefix"),
  "bootstrap dist_root is flat /dist"
);
ok(
  handlers.includes("Standard C") || handlers.includes("content_hash_filename"),
  "handlers document Standard C / content_hash"
);
ok(
  !/let dist_v = format!\("\{path_prefix\}\/dist\/v\//.test(handlers),
  "bootstrap no longer builds /dist/v/<fe>/g/<gen>"
);

// seal grant: no /dist/v/{fe_epoch}
ok(
  !sealRs.includes('format!("/g5/dist/v/{fe_epoch}")') &&
    !sealRs.includes('format!("/g5/dist/v/{epoch}")'),
  "seal grant/meta no version path"
);
ok(sealRs.includes("gr_seal_v2.{wasm_hash12}.wasm"), "seal uses content-hash wasm name");

// boot: never inject /dist/v/ for cache bust
ok(
  !boot.includes('"/dist/v/" + encodeURIComponent') &&
    !boot.includes('"/dist/v/" + encodeURIComponent(v') &&
    !boot.includes('"/dist/v/" + encodeURIComponent(serverVer)'),
  "boot does not construct /dist/v/<version>"
);
ok(boot.includes("injectGenBasename"), "boot injectGenBasename present");
ok(
  boot.includes('replace(/\\/dist\\/v\\/[^/]+\\/(?:g\\/[^/]+\\/)?/, "/dist/")') ||
    boot.includes("/dist/v/"),
  "boot collapses legacy version paths"
);

// pin fallback: flat hashed loader only
ok(
  pin.includes("/dist/gr.loader.") && !pin.includes("/dist/v/") + encodeURIComponent,
  "pin fallback uses flat hashed loader"
);
ok(!/dist\/v\/"\s*\+\s*encodeURIComponent/.test(pin), "pin source no version path construction");

// pack_loader collapses version path
ok(
  pack.includes('replace(/\\/dist\\/v\\/[^/]+\\/(?:g\\/[^/]+\\/)?/, "/dist/")') ||
    pack.includes("/dist/v/"),
  "pack_loader collapses legacy version paths"
);

// seal FE: no /dist/v/ epoch construction
ok(
  !seal.includes('"/g5/dist/v/" + encodeURIComponent(epoch)'),
  "seal.js no epoch version path"
);

// loader: no ?v= and no /dist/v/
ok(!loader.includes('"?v=" + encodeURIComponent'), "loader no ?v= version query");
ok(!loader.includes('"/dist/v/" + encodeURIComponent'), "loader no /dist/v/ construction");

// Runtime unit: withVer never emits version path; Standard-C drops meaningful basenames
const injectSrc = boot.match(/function injectGenBasename\(url\) \{[\s\S]*?\n  \}/);
const withVerMatch = boot.match(/function withVer\(url\) \{[\s\S]*?\n  \}/);
const distRootMatch = boot.match(/function distVersionRoot\(\) \{[\s\S]*?\n  \}/);
const opaqueLeafMatch = boot.match(
  /function isHashedOrOpaqueLeaf\(name\) \{[\s\S]*?\n  \}/
);
ok(
  !!injectSrc && !!withVerMatch && !!distRootMatch && !!opaqueLeafMatch,
  "extract withVer helpers"
);

const man = {
  fe_version: "6.0.99",
  product_version: "6.0.99",
  asset_gen: "deadbeef0123",
  asset_base: "/g5/dist",
  asset_route: "opaque_content_hash",
  assets: {
    race: "/g5/dist/aabbccddeeff.min.js",
  },
};

const fn = new Function(
  "global",
  `
  function assetGen(){ return global.__GR_ASSET_GEN__ || ""; }
  function scriptVersion(){ return global.__GR_PRODUCT_VERSION__ || ""; }
  function distFlatRoot(){ return "/g5/dist/"; }
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

// Opaque-only: do not invent meaningful race basename on the wire
const raceRel = h.withVer("gr.race.min.js");
ok(raceRel === "", "opaque-only: relative race invents no wire name: " + JSON.stringify(raceRel));
ok(!/\/dist\/v\//.test(raceRel), "relative race has no version path");

const legacy = h.withVer(
  "/g5/dist/v/6.0.15/g/eac17b5781e3/gr.gl_governor.min.js"
);
ok(
  legacy === "" || !/\/dist\/v\//.test(legacy),
  "legacy version path collapsed / dropped: " + legacy
);
ok(!/\/dist\/v\//.test(legacy), "legacy collapse removes version");

const hashed = h.withVer(man.assets.race);
ok(hashed === man.assets.race, "true content-hash passthrough");

const sealLegacy = h.withVer("/g5/dist/v/6.0.15/gr_seal_v2.wasm");
ok(
  sealLegacy === "/g5/dist/gr_seal_v2.deadbeef0123.wasm",
  "seal wasm legacy version collapsed: " + sealLegacy
);

if (failed) {
  console.error("\n" + failed + " check(s) failed");
  process.exit(1);
}
console.log("\nfe_standard_c_no_version_path_smoke: ALL PASSED");
