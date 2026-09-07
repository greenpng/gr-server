#!/usr/bin/env node
/**
 * Standard B smoke: pin fixed name only; boot/pack_loader/pin must not load
 * bare fixed basenames (must inject content-hash / asset_gen into filename).
 */
"use strict";

const fs = require("fs");
const path = require("path");
const assert = require("assert");

// 仓库根解析: 向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
let ROOT = __dirname;
while (ROOT !== path.parse(ROOT).root && !fs.existsSync(path.join(ROOT, "Cargo.toml"))) ROOT = path.dirname(ROOT);
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

// --- static: helpers present in boot source ---
const boot = read("gr.boot.js");
ok(boot.includes("function injectGenBasename"), "boot has injectGenBasename");
ok(boot.includes("function manifestAssetUrl"), "boot has manifestAssetUrl");
ok(boot.includes("content-hash") || boot.includes("a-f0-9]{8,16}"), "boot withVer preserves hashed URLs");
ok(
  /function assetUrl\(rel\)[\s\S]*manifestAssetUrl/.test(boot),
  "assetUrl prefers manifest"
);

// ensurePrivacyGuard must not hardcode bare ./gr.privacy_guard without hash gate
ok(
  !/return\s+"\.\/gr\.privacy_guard\.min\.js"/.test(boot),
  "privacy guard no bare ./ fixed fallback"
);
ok(
  boot.includes("No gen/manifest yet") || boot.includes("skip load; race pack"),
  "privacy guard skips until hashed URL available"
);

// pin source: no flat /dist/gr.loader.min.js bare fallback
const pinSrc = read("gr.js");
ok(
  pinSrc.includes("no_hashed_loader_fallback") ||
    pinSrc.includes("Only hashed loader"),
  "pin refuses bare loader fallback"
);
ok(
  !/A\s*\+\s*"\/dist\/gr\.loader\.min\.js"/.test(pinSrc),
  "pin does not construct flat fixed loader path"
);

// pack_loader prefers layers + withGenFilename
const pl = read("pack_loader.js");
ok(pl.includes("withGenFilename"), "pack_loader has withGenFilename");
ok(pl.includes("pack_url_template"), "pack_loader uses pack_url_template");
ok(
  pl.includes("content-hash") || pl.includes("a-f0-9]{8,16}"),
  "pack_loader keeps hashed URLs"
);

// ship artifacts stamped
const ver = fs.readFileSync(path.join(ROOT, "VERSION"), "utf8").trim();
for (const f of [
  "gr.boot.min.js",
  "gr.entry.min.js",
  "gr.race.min.js",
  "gr.min.js",
  "gr.pin.js",
  "pack_loader.min.js",
]) {
  const body = read(f);
  ok(body.includes(`__GR_BUILD_IMPL__="${ver}"`) || body.includes(`__GR_BUILD_IMPL__='${ver}'`), `${f} stamped ${ver}`);
  ok(body.length > 1000, `${f} non-trivial size (${body.length})`);
}

// entry must embed Standard-B boot logic (minified may mangle names; check regex tokens)
const entry = read("gr.entry.min.js");
ok(
  entry.includes("8,16") || entry.includes("[a-f0-9]{8,16}"),
  "entry.min embeds content-hash token regex"
);
ok(entry.includes("__GR_BOOT_STARTED__"), "entry.min embeds boot");
ok(entry.includes("GRPackLoader") || entry.includes("pack_loader"), "entry has pack loader");

// --- runtime unit: injectGenBasename / withVer logic via Function sandbox ---
const injectSrc = boot.match(
  /function injectGenBasename\(url\) \{[\s\S]*?\n  \}/
);
ok(!!injectSrc, "extract injectGenBasename from boot");

const withVerMatch = boot.match(/function withVer\(url\) \{[\s\S]*?\n  \}/);
ok(!!withVerMatch, "extract withVer from boot");
const opaqueLeafMatch = boot.match(
  /function isHashedOrOpaqueLeaf\(name\) \{[\s\S]*?\n  \}/
);
ok(!!opaqueLeafMatch, "extract isHashedOrOpaqueLeaf from boot");

const distRootMatch = boot.match(/function distVersionRoot\(\) \{[\s\S]*?\n  \}/);
const assetGenMatch = boot.match(/function assetGen\(\) \{[\s\S]*?\n  \}/);
const scriptVerMatch = boot.match(/function scriptVersion\(\) \{[\s\S]*?\n  \}/);

// Minimal harness re-implementing inject + path rules for pure unit checks
function makeHarness(manifest) {
  const global = {
    __GR_MANIFEST__: manifest,
    __GR_ASSET_GEN__: manifest.asset_gen,
    __GR_ASSET_BASE__: manifest.asset_base,
    __GR_PRODUCT_VERSION__: manifest.fe_version,
    __GR_SERVER_PRODUCT_VERSION__: manifest.product_version || manifest.fe_version,
  };
  // eslint-disable-next-line no-new-func
  const fn = new Function(
    "global",
    `
    ${assetGenMatch ? assetGenMatch[0] : "function assetGen(){return global.__GR_ASSET_GEN__||'';}"}
    function scriptVersion(){ return global.__GR_PRODUCT_VERSION__ || ""; }
    function distVersionRoot(){
      if (global.__GR_ASSET_BASE__) return String(global.__GR_ASSET_BASE__).replace(/\\/?$/, "/");
      return "/g5/dist/";
    }
    ${opaqueLeafMatch ? opaqueLeafMatch[0] : "function isHashedOrOpaqueLeaf(){return false;}"}
    ${injectSrc[0]}
    ${withVerMatch[0]}
    return { withVer: withVer, injectGenBasename: injectGenBasename, distVersionRoot: distVersionRoot, isHashedOrOpaqueLeaf: isHashedOrOpaqueLeaf };
  `
  );
  return fn(global);
}

const man = {
  fe_version: "6.0.20",
  product_version: "6.0.20",
  asset_gen: "deadbeef0123",
  asset_base: "/g5/dist",
  asset_route: "opaque_content_hash",
  assets: {
    // Standard C pure opaque basename
    race: "/g5/dist/aabbccddeeff.min.js",
  },
};

const h = makeHarness(man);

// pure opaque URL unchanged
assert.strictEqual(
  h.withVer(man.assets.race),
  man.assets.race,
  "pure opaque URL passthrough"
);

// embedded content-hash still passthrough
assert.strictEqual(
  h.withVer("/g5/dist/gr.race.aabbccddeeff.min.js"),
  "/g5/dist/gr.race.aabbccddeeff.min.js",
  "embedded content-hash URL passthrough"
);

// Standard C: meaningful product basenames must not be invented on the wire
const raceRel = h.withVer("gr.race.min.js");
ok(raceRel === "", "opaque-only: relative race invents no wire name: " + JSON.stringify(raceRel));

const flat = h.withVer("/g5/dist/gr.race.min.js");
ok(flat === "", "opaque-only: flat meaningful basename dropped: " + JSON.stringify(flat));

const fixedUnderVer = h.withVer(
  "/g5/dist/v/6.0.16/g/deadbeef0123/gr.race.min.js"
);
ok(
  fixedUnderVer === "" || fixedUnderVer.indexOf("/dist/v/") < 0,
  "legacy version path collapsed (no leak): " + fixedUnderVer
);

// bootstrap assets secondary keys (server) — static check handlers.rs
const handlers = fs.readFileSync(
  path.join(CRATES, "gr-probe-plane/src/handlers.rs"),
  "utf8"
);
ok(handlers.includes('"privacy_guard"'), "bootstrap assets.privacy_guard");
ok(handlers.includes('"probe_self_heal"'), "bootstrap assets.probe_self_heal");
ok(handlers.includes("opaque_content_hash"), "asset_route opaque_content_hash");
ok(handlers.includes("opaque_public_filename"), "opaque_public_filename helper");

// ASSET_GEN file present
const gen = fs.readFileSync(path.join(FE, "ASSET_GEN"), "utf8").trim();
ok(/^[a-f0-9]{8,16}$/i.test(gen), "ASSET_GEN is hex token: " + gen);

if (failed) {
  console.error("\n" + failed + " check(s) failed");
  process.exit(1);
}
console.log("\nfe_standard_b_hashed_only_smoke: ALL PASSED");
