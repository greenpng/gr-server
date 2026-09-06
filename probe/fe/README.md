# FE — first-pack / sub-pack / sandbox (v5)

Design truth: [`../10-fe-race-sandbox-design-from-v57.md`](../10-fe-race-sandbox-design-from-v57.md)

## Layout

| File | Role |
|------|------|
| `gr.boot.js` | First pack: static wave race → multi-tick analyze/applyRoutePlan loop |
| `pack_loader.js` | Async parallel kick + `applyRoutePlan` (skip already-kicked) + health |
| `upload_queue.js` | Sub-pack self-managed upload (conc/retry/pagehide) |
| `sandbox_tree.js` | `iframe` / `sandbox_iframe` / `worker` parallel |
| `nest_frame.html` | Nested sandbox page |
| `storage.js` | `localStorage` vt + cache-bust helpers |
| `collectors/registry.js` | Lite collectors (B0–B3, B8 early, B11, B7) — **not** full v4 161 yet |

## Semantics

- Priority = **kick order**, not finish-gate  
- Sub-packs upload themselves via `GRUploadQueue`  
- Boot watches load/run; does not wait for upload completion  
- `inject_path` ∈ `cf_worker|nginx|app` on open/ingest  

## Component honesty

| Tree | Count |
|------|-------|
| v4 logical components | **161** (`probe/.../component_catalog.rs`) |
| v57 business batches | ~17 (B0–B16) |
| v5 registered collectors now | see `GRCollectors.count()` (lite subset) |

## Local

```bash
cargo run -p gr-service -- --bind 0.0.0.0:28770 --db data/gr.sqlite
# static-host fe/ and open a page with:
# <script src="./gr.boot.js" data-endpoint="http://127.0.0.1:28770" data-inject-path="nginx"></script>
```
