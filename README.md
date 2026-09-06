# GR Server (Green V7)

Green V7 / GR 服务端公开发行仓：探测面 · 控制面 · 签名模块 · 管理面板。
本仓是**安装与版本更新的唯一公开入口**（`install.sh` / 更新脚本 / 面板 OTA 都指向本仓 Release）。

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

安装器会做 sha256 + ELF + ed25519 模块签名校验，落地 `/opt/green-v7`，
生成 `.env` 与 systemd 服务，并通过控制面/探测面 `/v1/health` 健康门。

## 更新（已装节点）

| 优先级 | 通道 | 适用 |
|--------|------|------|
| P0 | 管理面板 OTA（set-release-url → install / install-fe / install-runtime） | 默认 |
| P1 | `install/release/update_runtime_from_github.sh`、`update_module_from_github.sh` | 无面板 / 免费节点 |
| P2 | SSH 手工 | 进程死 / 首次装机 |

所有更新都拉本仓同一 tag 的 Release 资产。**Docker 只是运行时容器，不是更新通道。**

## 文档

- [docs/01-install.md](docs/01-install.md) — 安装（二进制 / Docker 数据层）
- [docs/02-deploy-probe-sdk.md](docs/02-deploy-probe-sdk.md) — 网站嵌入探测 + 后端 SDK
- [docs/03-modules.md](docs/03-modules.md) — 模块与热更
- [docs/04-operations.md](docs/04-operations.md) — 备份、升级、回滚
- [docs/05-local-dev.md](docs/05-local-dev.md) — 从源码构建与测试

## 从源码构建

```bash
cargo build --release -p gr-service -p gr-cli   # 需 rust-toolchain.toml 指定的工具链
bash tests/runners/run_v7_acceptance.sh contract  # 契约测试
```

正式发布产物只由本仓 CI（`gate-a.yml` → `release.yml`）打出并经 Gate B 安装复测后上传。

## 安全

- Release 资产由根 Ed25519 钥签署，安装器钉死公钥指纹（`install.sh` 内 `OTA_ROOT_PUBKEY_SHA256`）。
- 私钥只存在于 GitHub Secrets，永不入库；仓内 `ota_ed25519.pk` 是公钥。
- tag 与 Release 资产不可变：修复以新 PATCH 版本发布，不覆盖历史。
