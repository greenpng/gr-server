#!/usr/bin/env node
// gen_random_ondemand_packs.js — R100 pseudo-static pack 维护工具。
//
// 背景: data/r100_templates.json 是 R100 反脚本通道 (Rxx_spotcheck) 的
// 模板 SSOT — 100 个 pack, 每 pack 12 维度 × 每维度 N 个 anti-spoof 探针
// (词表为 mode×arg×surface 组合, ~8k 项)。本脚本从既有模板文件提取维度
// 词表, 以确定性种子重新采样 pack 组合 (定期轮换 pseudo-static 指纹面),
// 不新增词表项 — 词表演进 = 手工编辑输入文件后重跑。
//
// 用法:
//   node scripts/gen_random_ondemand_packs.js                 # seed=1 → 落盘 data/r100_templates.json
//   node scripts/gen_random_ondemand_packs.js --seed 7        # 换种子
//   node scripts/gen_random_ondemand_packs.js --out /tmp/t.json --count 50
//   node scripts/gen_random_ondemand_packs.js --dry-run       # 只验证不落盘
//
// 语义约束 (gr-probe-core r100_templates.rs):
//   - 顶层: {version, algo, mode, runtime, pack_count, packs}
//   - pack id: `R{ii}_spotcheck` (00..99), 条目 {s, p, dims:[{d, ops:[…]}]}
//   - s = 序号, p = 探针权重档 (22), dims 12 维固定
//   - 加载门槛: pack_count >= 50 (r100_templates.rs 契约测试)
// 确定性: 同 seed 同输入 → 逐字节同输出 (mulberry32; 维度间独立流)。
// 产品链: 文件随整包 data_tree 签名分发 (greenpng 1.0.8+); 本工具仅在
// 维护时人工运行, 产出经评审后提交。

"use strict";

const fs = require("fs");
const path = require("path");

function parseArgs(argv) {
  const a = { seed: 1, count: 100, out: null, in: null, "dry-run": false };
  for (let i = 2; i < argv.length; i++) {
    const k = argv[i];
    if (k === "--seed") a.seed = Number.parseInt(argv[++i], 10);
    else if (k === "--count") a.count = Number.parseInt(argv[++i], 10);
    else if (k === "--out") a.out = argv[++i];
    else if (k === "--in") a.in = argv[++i];
    else if (k === "--dry-run") a["dry-run"] = true;
    else if (k === "--help" || k === "-h") {
      console.log(fs.readFileSync(__filename, "utf8").split("\n").slice(1, 24).join("\n"));
      process.exit(0);
    } else {
      console.error(`unknown arg: ${k}`);
      process.exit(2);
    }
  }
  if (!Number.isInteger(a.seed) || a.seed <= 0) {
    console.error(`--seed must be a positive integer (got ${a.seed})`);
    process.exit(2);
  }
  if (!Number.isInteger(a.count) || a.count < 50 || a.count > 999) {
    console.error(`--count must be 50..999 (r100 load gate is >=50; got ${a.count})`);
    process.exit(2);
  }
  return a;
}

// mulberry32 — small deterministic PRNG (no deps; same output on node versions)
function mulberry32(seed) {
  let t = seed >>> 0;
  return function () {
    t = (t + 0x6d2b79f5) >>> 0;
    let r = Math.imul(t ^ (t >>> 15), 1 | t);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}
function hashStr(s) {
  // FNV-1a 32 — stable per-(seed,dim,pack) stream split
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function main() {
  const args = parseArgs(process.argv);
  const root = path.resolve(__dirname, "..", ".."); // 02-probe-analysis → greenpng root
  const inPath = path.resolve(args.in || path.join(root, "data", "r100_templates.json"));
  const outPath = path.resolve(args.out || inPath);

  const src = JSON.parse(fs.readFileSync(inPath, "utf8"));
  const packs = src.packs || {};
  const packIds = Object.keys(packs).sort();
  if (packIds.length < 50) {
    console.error(`input vocab file must carry >=50 packs (got ${packIds.length})`);
    process.exit(1);
  }

  // 1) 词表提取: dim → ops (保持出现序, 去重) + 每 dim 历史 ops/pack 基线
  const vocab = new Map(); // dim → [{mode,arg,zone,id}, …]
  const perPackCount = new Map(); // dim → 中位 ops/pack
  const dimOrder = [];
  const countsByDim = new Map();
  for (const pid of packIds) {
    for (const dim of packs[pid].dims || []) {
      if (!vocab.has(dim.d)) {
        vocab.set(dim.d, []);
        countsByDim.set(dim.d, []);
        dimOrder.push(dim.d);
      }
      const seen = vocab.get(dim.d);
      const key = new Set(seen.map((o) => o.id));
      for (const o of dim.ops || []) {
        if (!key.has(o.id)) {
          seen.push({ mode: o.mode, arg: o.arg, zone: o.zone, id: o.id });
          key.add(o.id);
        }
      }
      countsByDim.get(dim.d).push((dim.ops || []).length);
    }
  }
  for (const [d, cs] of countsByDim) {
    cs.sort((x, y) => x - y);
    perPackCount.set(d, cs[Math.floor(cs.length / 2)]);
  }

  // 2) 重采样: pack i, dim d → 确定性流取 n 个不重复词表项
  const newPacks = {};
  for (let i = 0; i < args.count; i++) {
    const id = `R${String(i).padStart(2, "0")}_spotcheck`;
    const dims = [];
    for (const d of dimOrder) {
      const ops = vocab.get(d);
      const n = Math.min(perPackCount.get(d), ops.length);
      const rnd = mulberry32((hashStr(`${args.seed}:${d}:${id}`) ^ 0x9e3779b9) >>> 0);
      // partial Fisher-Yates over vocab copy → 前 n 项为样本 (保持词表原序输出)
      const idx = ops.map((_, j) => j);
      for (let j = 0; j < n; j++) {
        const k = j + Math.floor(rnd() * (idx.length - j));
        [idx[j], idx[k]] = [idx[k], idx[j]];
      }
      const picked = idx.slice(0, n).sort((x, y) => x - y).map((j) => ops[j]);
      dims.push({ d, ops: picked });
    }
    newPacks[id] = { s: i, p: packs[packIds[0]].p ?? 22, dims };
  }

  const out = {
    version: src.version ?? 1,
    algo: src.algo,
    mode: src.mode,
    runtime: src.runtime,
    pack_count: args.count,
    packs: newPacks,
  };

  // 3) 自检: 与加载器同规 (pack 数门槛 + 每 pack 12 维 + ops 非空)
  const okPacks = Object.keys(out.packs).length;
  if (okPacks !== args.count) throw new Error("pack count mismatch");
  for (const [pid, pk] of Object.entries(out.packs)) {
    if (!pk.dims.length || pk.dims.some((d) => !d.ops.length)) {
      throw new Error(`pack ${pid} has an empty dim/ops`);
    }
  }

  const body = JSON.stringify(out, null, 1) + "\n";
  if (args["dry-run"]) {
    console.log(
      `dry-run ok: packs=${okPacks} dims=${dimOrder.length} vocab=${[...vocab.values()].reduce((s, v) => s + v.length, 0)} seed=${args.seed}`
    );
    return;
  }
  fs.writeFileSync(outPath, body);
  console.log(
    `written ${outPath}: packs=${okPacks} dims=${dimOrder.length} vocab=${[...vocab.values()].reduce((s, v) => s + v.length, 0)} seed=${args.seed}`
  );
}

main();
