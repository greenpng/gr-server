# FE collector domain modules

| File | Domain | Packs / helpers |
|------|--------|-----------------|
| `registry.js` | SSOT core + mid/hard packs | multipath residual, B0–B46, helpers |
| `registry.b10x.js` | **deepen** engine residual | `B10x_*` (load after registry) |
| `registry.static*.js` | progressive split | auto from `split_registry.py` |
| `registry.mid.*.js` | mid progressive | auto-split |
| `registry.dense.js` | dense B47+ | auto-split |
| `registry.random*.js` | verify_rand R00–R99 | authenticity lane |
| `l1.js` | lite race | B0–B3 early |

## Load order (boot)

1. race (queue + l1)  
2. registry static lite → hard  
3. **registry.b10x.js** (after hard — needs multipath helpers)  
4. mid / dense / random on demand  

## Pack lanes

- `core_identity` — B0–B3, B8, B10, B11, B12  
- `deepen` — B10x_*, dense, mid physical  
- `verify_rand` — R*  

See `v5-docs/architecture/pack-budget-tiers.md`.
