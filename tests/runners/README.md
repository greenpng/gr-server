# Green V7 GitHub Runner 测试

统一入口：

```bash
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh contract
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh frontend
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh business
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh multi-node
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh upgrade
bash 03-local-test-lab/tests/runners/run_v7_acceptance.sh ota-negative
```

根目录 `.github/workflows/` 是私有源码级 workload，公开仓库 `greenpng/install`
的 `.github/workflows/` 是安装包测试（免费公共 runner）：

- `greenpng/install test-install-sh.yml`：x86_64/aarch64 上 `install.sh --env-file`
  非交互安装（4 库就绪等待）→ 冒烟。
- `greenpng/install test-install-docker.yml`：`install.sh --with-docker` 安装 →
  冒烟。
- `greenpng/install test-install-upgrade.yml`：升级/版本控制契约
  （`test/upgrade_rollback_check.sh`，本地 fixture；即本目录脚本的镜像副本）。
- `03-local-test-lab/tests/runners/ota_negative_check.sh`（`ota-negative` 入口）：V7 安装器/更新器
  manifest 校验负面用例——篡改 manifest、缺失/无效签名、根公钥替换、asset hash
  篡改、架构不匹配、V7 主版本强制。需 OTA 根私钥（`keys/ota_ed25519.sk`）签名
  fixture，缺钥环境自动 skip。
- 私有 `v7-contract.yml`：Rust/官网/面板/FE 契约；`v7-package.yml`：SLA/打包校验；
  `v7-business.yml`：安装后业务全流程；`v7-multi-node.yml`：多节点/运维验收；
  `v7-panel-ota.yml`：面板在线 OTA。以上仅 workflow_dispatch + 周定时（避免
  私有 runner 计费分钟）。
- `v7-install.yml` / `v7-upgrade.yml` 已移除（职责移交公开仓免费 runner）。

`04-release-github-ci/install/.github/workflows/` 与公开仓库通过 `04-release-github-ci/release/sync_public_install.sh` 同步。
`04-release-github-ci/install/.github/workflows/` 保留为安装仓库的同步副本；提交时以根目录 workflow
为准（仅 dispatch 触发），公开安装测试以 `greenpng/install` 为准。

详见 `docs/testing/03-RUNNER-TESTS.md` 和 `03-local-test-lab/tests/local/prodsim_full_suite.sh`。
