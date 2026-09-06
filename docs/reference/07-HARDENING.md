# 07 · 反破解加固（build_id + 每版本密钥轮换 + 字符串混淆）

P0+P1（用户选型：P0 全部 + P1 的模块自校验与 FE 混淆；OTA 不做版本限制，
证书链验证已够）。目标：**每个发布自动交叉**——上一版本的破解/补丁不能复用。

## 威胁模型与设计要点

| 项 | 旧状态 | 现状 |
|---|---|---|
| 字符串可识别 | `strings` 直接看到 `grlic1`、签名域前缀等 | `gr-obf` 编译期 XOR（每版本盐），`strings`/`grep` 不可见 |
| 二进制可复用 | 无 build_id，A 版二进制可伪装成 B 版 | `GR_BUILD_ID` 每版本随机，写入二进制 + manifest + FE stamp |
| 签名密钥轮换 | 单一长期 OTA key（CI secret） | 每版本临时 release key，根 key 签发证书（`gr sign-cert`），模块双签 `sig`+`sig2` |
| 本地模块完整性 | 无 | 启动时 `verify_active_modules`：活动模块链签 + sha256 自检 |
| FE JS | 仅 terser 压缩 | `javascript-obfuscator`，每发布种子（build_id 派生），字符串数组 + 16 进制标识符 |

用户明确不做：OTA 版本号限制（证书链验证足够，历史版本仍可升级）。

## 每发布流程产物（build_and_publish.sh / build_multiarch.sh 自动做）

1. `GR_BUILD_ID=$(openssl rand -hex 12)` → 导出给全部 crate（build.rs 注入），
   写入 `dist/release-<V>/build_id` 与 manifest `build_id` 字段。
2. `GR_OBF_SALT=$(openssl rand -hex 16)` → 各 crate build.rs 派生 64 字符
   XOR key；`02-probe-analysis/crates/gr-obf` 的 `obf![]` 编译期加密受盖字符串。
3. 每版本临时密钥对 `dist/release-<V>/keys-release/`：
   - `gr keygen` → `ota_ed25519.sk/.pk`（临时，**不发布**）；
   - `gr sign-cert --secret-key keys/ota_ed25519.sk --version <V> --build-id <BID> --release-pubkey <hex>`
     → 根 key 签发 `release_cert`（消息域 `gr-release-cert-v1|version|build_id|pubkey`）；
   - `sign-module --release-key <临时 sk>` → 每个模块 `sig`（根）+ `sig2`（发布 key）；
   - `sign-manifest --build-id --release-pubkey --release-cert` → manifest 绑定三者。
4. FE：`rebuild_bundles.sh` 以 `seed=sha256(build_id|version)[:12]` 跑
   javascript-obfuscator（`--rename-globals false` 保全局、`--self-defending false`），
   版本 stamp `window.__GR_BUILD_IMPL__` 保持有效。

## 运行时验证链（gr-ota）

- `fetch_manifest_blocking`（`&mut self`）→ `verify_manifest_chain`：
  根 sig 校验 manifest 体；若带 `release_pubkey` 再验 `release_cert`，
  成功后把 release key 存入 `cfg.release_key`，并 `write_release_binding`
  持久化到 `modules_dir/release_binding.json`（boot 离线可用）。
- `stage_local_file` → `verify_artifact_sig_chain`：根 sig（旧版兼容）+ 
  绑定 release key 时必须存在且通过 `sig2`。
- 旧发布（无 release 字段）仍走根 key 单签路径，兼容不阻断。
- 服务启动（gr-service main）：pubkey 存在时执行 `ota_verify_active_modules`
  —— 遍历 `active/<name>` → `versions/<name>/<v>/meta.json`（签名工件）+
  重哈希 `.so`。失败：prod/`GR_STRICT_BOOT_VERIFY=1` 直接拒绝启动；lab 仅告警。
  无 meta.json 的旧模块列为 `legacy_unverifiable`，不误杀存量安装。

## 本地验证命令

```bash
# 1) 发布（HOST_ONLY=1 单架构；SKIP_PUBLISH=1 不对外）
HOST_ONLY=1 SKIP_PUBLISH=1 bash 04-release-github-ci/release/build_multiarch.sh

# 2) 整树验证：manifest 链 + 每个模块链签 + sha256（测试跳过条件：无环境变量）
VERSION="$(cat VERSION)"
GR_RT_MANIFEST="dist/release-${VERSION}/manifest-x86_64-linux-gnu.json" \
GR_RT_PUBKEY=keys/ota_ed25519.pk \
GR_RT_DIR="dist/release-${VERSION}" \
cargo test -p gr-ota --test release_tree_verify -- --nocapture

# 3) 字符串抽查（期望 0 命中）
strings "dist/release-${VERSION}/gr-service-${VERSION}-x86_64-linux-gnu" | grep -c grlic1
strings "dist/release-${VERSION}/gr-service-${VERSION}-x86_64-linux-gnu" | grep -c gr-release-cert-v1
02-probe-analysis/probe/fe/gr.race.min.js 内不应再出现明文 "gr-challenge-bind-v2|"
```

## 增量发布（publish_modules.sh / build_signed_modules.sh）

- 同版本增量**复用**同一发布 key + build_id（脚本从 `dist/release-<V>/` 读取），
  节点已绑定的 release binding 继续有效；manifest 合并时保留
  build_id/release_pubkey/release_cert 并重新签名，不允许换 key 覆盖旧 manifest。
- `--release-key` 缺省 / manifest 字段丢失的旧流程已修复：`strings` 层面的夹具不再
  泄漏新格式（详见脚本注释）。
- 增量后验证：`release_tree_verify`（manifest 链 + 每模块 sig2 + sha256）。

## Runner 测试（GitHub runner）

- 私有 green-v7 `release-multiarch.yml`：workflow_dispatch(publish=false) 验证双架构
  加固构建（prepare build_id、FE 混淆、sign-cert、双签、manifest 绑定）。
- **发行仓 greenpng/gr-server**（公开 runner 免费）`test-install-sh.yml` /
  `test-install-docker.yml`：安装器从本仓库 release 下载资产（sha256 校验）的
  干净环境安装 + 冒烟；asset 下载不受 manifest 新字段影响（install.sh 只读取
  runtime/fe 的 sha256 + asset 字段）。`test-install-upgrade.yml` 覆盖升级/版本控制。
- 私有 green-v7 `v7-panel-ota.yml`（面板 OTA）与源码级套件仅手动 dispatch / 周
  定时触发，不随 push/PR 自动跑（私有 runner 计费；见 docs/testing/03-RUNNER-TESTS.md）；
  镜像同步用 `bash 04-release-github-ci/release/sync_public_install.sh`。
- 注意：**旧版本运行时**解析新格式 manifest 时，其 sign 校验体不含
  build_id/release 字段 → 验签失败 → 安全拒绝升级（符合“证书链验证即可”的设计，
  旧二进制不能吃到带轮换签名的新包）。

## 密钥处置规则

- `keys/ota_ed25519.sk`（根）只存在于发布机器/CI secret `GR_OTA_SIGNING_KEY`；
- 每版本 release key 只存在于 `dist/release-<V>/keys-release/`（本地），
  发布清单 `find -maxdepth 1` 保证其不随资产上传播；
- 环境变量 `GR_BUILD_ID` / `GR_OBF_SALT` 可覆盖以便复现（CI 用 prepare job
  生成一次 build_id，双架构共享同一身份）。

## 已覆盖/未覆盖

- 覆盖：二进制字符串混淆、build_id、每版本 key 轮换 + 证书链、模块双签、
  boot 自校验、FE 全量混淆。
- 未覆盖（已知边界）：`gr.seal.js` 的 wire 协议常量（suite_id/wasm id 等）
  保持明文——它们在 FE wasm/panel/edge 间共享，仅混淆 Rust 侧无意义；
  XOR 混淆 + js-obfuscator 属于**提高破解成本**，非形式化安全边界
  （`.grm` 加壳默认关闭，见 `ALLOW_INSECURE_GRM`）。

## 运行时与隐私默认（v7）

- **发布物签名校验**：模块/FE/运行时/full-upgrade 校验 manifest、证书链和
  artifact 签名；所有安装均可使用完整功能，不再有付费许可门。回退与保留版本见
  **05-RELEASE.md**（激活版 + 最近 3 版）。
- **站点 consent 门（管理面）**：探针隐私只在管理面确认——新建站点必须由网站主
  勾选“探测行为已告知访客”（`consent_confirmed=true`），否则 400 `consent_required`；
  更新站点不重复确认；官网导入视为已确认配对。库表 `control.sites` 增列
  `consent_confirmed_at` / `consent_notice_version`（旧库自动 ALTER 补列）。
- **配额**：站点、保留期、节点和会话使用量由管理员配置或部署策略控制；
  不依赖订阅 token。探针分析响应仍附带 `usage_monthly` 等运维统计。

## 根密钥轮换记录

| 日期 | 事件 | 公钥 sha256 | 地址 |
|---|---|---|---|
| 2026-08-20 | 根密钥首次轮换（旧钥疑似暴露于调试日志） | 旧 `2558e6b3409ab33b8cb8c5c3dc9861ae795a43ea389830abe70b08ce4e11aa47` | 旧钥已归档 `/home/ubuntu/key-archive/`（本仓库外，0600） |
| 2026-08-20 | 新根密钥生效（自下一个发布起） | 新 `6dffd7d3c40b75a21db766cbee97ddebef21bcf089e30ba6dea0db5d8a1754a2` | `keys/ota_ed25519.sk/.pk` + CI secret `GR_OTA_SIGNING_KEY`（base64） |

- 安装器/运行时验签使用的公钥取自**每个 release 自带的 `ota_ed25519.pk` 资产**，轮换无需迁移已部署节点：旧发布保留旧公钥资产，新发布携带新公钥（链自洽）。
- v6.0.28 旧签名格式由新运行时**安全拒绝**（`BadSignature`，manifest 消息格式跨代漂移）——迁移时按旧 manifest 自身 sha256 校验资产完整性。
- 为旧发布做增量重签（`publish_modules.sh`）必须继续用旧根钥（归档副本），否则链断裂。
