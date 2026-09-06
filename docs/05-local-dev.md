# 从源码构建与测试（开发者）

```bash
# 工具链: rust-toolchain.toml 指定的 Rust 版本
cargo build --release -p gr-service -p gr-cli

# 契约测试(单测 + check + 面板构建) / FE 静态契约
bash tests/runners/run_v7_acceptance.sh contract
bash tests/runners/run_v7_acceptance.sh frontend

# 本地预演打包(实验室钥, 产物仅预演, 不得发布)
GR_ALLOW_KEYGEN=1 SKIP_PUBLISH=1 HOST_ONLY=1 bash release/build_multiarch.sh
```

正式发布只走本仓 CI: `gate-a.yml` 全绿 → `release.yml`(生产钥打包 → Gate B
安装/升级复测 → tag + Release)。本机 dist/ 永远不是用户资产。
