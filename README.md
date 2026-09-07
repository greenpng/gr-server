# GR Server (greenpng)

GR 服务端发行仓：安装 · 探测 · 分析 · 返回 · SDK · 签名模块 · 管理面板。
本仓是 **02 探测分析产品线的整理发布打包仓**，也是**安装与版本更新的唯一公开入口**
（`install.sh` / 更新脚本 / 面板 OTA 都指向本仓 Release）。

- 版本：见 [`VERSION`](VERSION)（当前 8.0.x）
- 许可：MIT（见 [LICENSE](LICENSE)）
- 在线更新通道：本仓 GitHub Release + 仓库内 `install/` 脚本

## 安装（新节点）

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# 或显式版本 / 架构：
bash install/install.sh --version 8.0.3 --arch x86_64 --yes
# 本机无 PostgreSQL/Redis 时（只起数据层，本体仍是宿主机二进制）：
bash install/install.sh --version 8.0.3 --with-docker --yes
```

安装器会做 sha256 + ELF + ed25519 模块签名校验，落地 `/opt/greenpng`，
生成 `.env` 与 systemd 服务，并通过控制面/探测面 `/v1/health` 健康门。

## 更新（已装节点）

| 优先级 | 通道 | 适用 |
|--------|------|------|
| P0 | 管理面板 OTA（set-release-url → install / install-fe / install-runtime） | 默认 |
| P1 | `install/release/update_runtime_from_github.sh`、`update_module_from_github.sh` | 无面板 / 免费节点 |
| P2 | SSH 手工 | 进程死 / 首次装机 |

所有更新都拉本仓同一 tag 的 Release 资产。**Docker 只是运行时容器，不是更新通道。**

## 仓库内容

| 目录 | 内容 |
|------|------|
| `crates/` | Rust 工作区：gr-service（控制/探测/网关）、gr-probe-core、gr-probe-plane、gr-ota 等 |
| `modules/` | 签名热更模块源（identity / brain / analyze / ingest / edge / probe_assets） |
| `probe/` | 浏览器探测 FE（gv5.seal.js 打包链, sealed ingest） |
| `panel/` | 管理面板（admin-ui 源 + admin-spa 产物） |
| `sdk/` | 后端接入 SDK（探测数据回传/结果拉取） |
| `spec/` | 线协议/评分规格（运行时加载, 非文档） |
| `fixtures/` | 契约测试数据 |
| `scripts/` | 构建脚本与 FE 工具链（含 `scripts/fe/checks/` FE 静态契约检查） |
| `vendor/` | vendored 依赖源（pingora） |
| `install/` | 安装器 + 数据层 compose + 升级脚本 |
| `release/` | 打包脚本（build_multiarch / SBOM / 模块签名） |

文档、实验室测试与官网在开发仓维护，不进入本发行仓。

## 从源码构建与测试

```bash
# 工具链: rust-toolchain.toml 指定的 Rust 版本
cargo build --release -p gr-service -p gr-cli

# 契约测试（= CI Gate A 同款）
cargo test -p gr-probe-store --lib
cargo test -p gr-probe-plane --lib
cargo check -p gr-service -p gr-admin
npm ci --prefix panel/admin-ui && npm run build --prefix panel/admin-ui

# FE 静态契约检查
for f in scripts/fe/checks/*.js; do node "$f"; done
```

正式发布产物只由本仓 CI（`gate-a.yml` → `release.yml`）打出并经 Gate B 安装复测后上传。

## 安全

- Release 资产由根 Ed25519 钥签署，安装器钉死公钥指纹（`install.sh` 内 `OTA_ROOT_PUBKEY_SHA256`）。
- 私钥只存在于 GitHub Secrets，永不入库；仓内 `ota_ed25519.pk` 是公钥。
- tag 与 Release 资产不可变：修复以新 PATCH 版本发布，不覆盖历史。
