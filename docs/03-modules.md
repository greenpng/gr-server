# 04 · 模块 ↔ 打包 .so 映射

主程序按「核心 crates + 插件模块」组织。6 个插件模块各打一个签名 `.so`，
发布到 `greenpng/gr-server`（GitHub Release），管理面板按模块 OTA 热更。

## 映射表

| `02-probe-analysis/modules/` 目录 | crate | 打包资产 | 业务域 |
|---|---|---|---|
| `identity/` | gr-module-identity | `libgr_identity-<v>-<triple>.so` | 身份 + 站点级加密矩阵 / challenge-seal 隔离 |
| `brain/` | gr-module-brain | `libgr_brain-<v>-<triple>.so` | 大脑调度 / 分析编排 |
| `analyze/` | gr-module-analyze | `libgr_analyze-<v>-<triple>.so` | 算法分析 |
| `ingest/` | gr-module-ingest | `libgr_ingest-<v>-<triple>.so` | 探测数据接入 |
| `02-probe-analysis/edge/` | gr-module-edge | `libgr_edge-<v>-<triple>.so` | 网关 / 边界 |
| `probe-assets/` | gr-module-probe-assets | `libgr_probe_assets-<v>-<triple>.so` | 探测 FE 资产: 固定入口 + 版本化子资源 + 缓存 meta |

`<triple>` = `x86_64-linux-gnu` / `aarch64-linux-gnu`。

## 构建签名

```bash
# 全部模块 (release 布局, 不发布):
bash 04-release-github-ci/release/build_multiarch.sh SKIP_PUBLISH=1
# 增量 (只编签指定模块 → 面板 install 该模块):
bash 04-release-github-ci/release/publish_modules.sh analyze
```

- 签名密钥：`keys/ota_ed25519.sk`（不入库）；公钥 `keys/ota_ed25519.pk`，
  随发布资产分发 `ota_ed25519.pk`。
- 每个模块同时产出 `<name>.artifact.json`（sign-module 签名信封），manifest
  记录 runtime/FE sha256 + modules（签名）。全部经 `gr-cli sign-module` /
  `sign-manifest`。

## 运行时装载

- `gr-service --modules-dir <dir>`（运行时 staging 在 `data/*/modules`，
  不是仓库根 `02-probe-analysis/modules/`——后者是源码）。
- OTA 激活路径：管理面板 set-release-url → ota/install（按模块）；或本机
  `gr-cli module stage <artifact.json> <so>` + `gr-cli module activate <name> <version>`。
- 热更语义：模块→so 热更（不重启）；FE→install-fe；plane/runtime→install-runtime+重启。

## 版本一致性

- 模块的 `GR_MODULE_VERSION` 与仓库 `VERSION`（当前 8.0.2）保持一致；
  `04-release-github-ci/release/build_multiarch.sh` 的版本 SSOT 校验覆盖
  `VERSION` / `02-probe-analysis/probe/fe/VERSION` / Cargo workspace / `VERSION.probe` 四处。
