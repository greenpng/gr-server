# green-v6 更新与部署规则（强制）

> **SSOT**：本文件为发版/部署行为约束。会话、脚本、运维一律按此执行。  
> 关联：`docs/PANEL_HOT_UPDATE_ALL.md`（面板 API 细节）。

---

## 0. 一句话

| 优先级 | 方式 | 何时用 |
|--------|------|--------|
| **P0 主路径** | **管理面板 OTA** | 默认一切更新 |
| **P1 兜底** | **SSH**（178:21617） | 仅当面板不可用：进程挂死、控制面起不来、首次装机、密钥/基建 |

禁止把「SSH 从 GitHub 直装」当成默认习惯；面板不是备选，是主用。

### 0.1 生产节点环境（178 等 nginx 前置）

`≥6.0.9` 起生产默认 **拒绝 `0.0.0.0` 公网 bind**。若进程仍直接监听公网（前置 nginx 反代本机），必须在 `.env` 设置：

```bash
GR_ALLOW_PUBLIC_BIND=1
GR_DEPLOY_ENV=prod
GR_RELEASE_URL=https://github.com/kullyeilert-jpg/gr-releases/releases/download/vX.Y.Z
GR_PUBKEY_PATH=/opt/green-v6/keys/ota_ed25519.pk
```

缺少 `GR_ALLOW_PUBLIC_BIND=1` 时 runtime OTA 重启会 **exit 1 起不来**。

### 0.2 完整 release manifest（≥6.0.9）

`scripts/release/build_and_publish.sh` 产出：

- `runtime.asset` + `runtime.sha256`
- `fe.asset` + `fe.sha256`（FE 包 **tar -h 解引用 symlink**）
- 各 module ed25519 **v2** 签名
- **manifest.sig**（Ed25519 over canonical body）

面板 `install-fe` / `install-runtime` 会按 manifest 验哈希；运行中二进制安装用 **临时文件 + rename**（避免 ETXTBSY）。

---

## 1. 角色分工

```
本机 lab（/home/ubuntu/green-v6）
  ├─ 改代码 / 测通
  ├─ 按变更范围 打包 + 签名
  └─ 发布到 GitHub Releases（仅变更资产）
           │
           ▼
管理面板（178 控制面 /{console}/）
  ├─ set-release-url → 指向新 tag（或同 tag 增量资产）
  ├─ install 指定模块 / install-fe / install-runtime
  └─ 验证 health + 抽样 session
           │
           ▼  （仅失败时）
SSH 兜底：诊断 / 拉起进程 / 紧急 systemctl
```

- **构建机**：出包、签名、上传 GitHub。  
- **178 拉包**：**由面板触发进程内下载**（节点访问 GitHub），不是构建机 scp 业务资产为默认。  
- **SSH**：运维急救，不替代日常 OTA。

---

## 2. 变更分类 → 打包与更新范围（禁止无脑全量）

| 变更类型 | 典型路径 | 构建产物 | 面板动作 | 是否重启 |
|----------|----------|----------|----------|----------|
| **A. 单模块算法/业务** | `crates/gr-module-analyze` 等 | **仅**对应 `libgr_<name>-{ver}-*.so` + 签名 + 更新 manifest 中该条 | `set-release-url`（如有新 tag）→ **`ota/install` 该 name** | **否**（hot_load；analyze 会 mint_wire） |
| **B. 多模块但非 runtime** | 若干 module crates | **仅**变更的 .so 集合 + manifest 中对应条目 | 对每个 name `install`，或 install-all **仅当**本 tag 模块集=本轮变更集 | **否** |
| **C. FE 探针/SDK 静态** | `fe/**` | `fe-{ver}.tgz`（或后续更细粒度 FE 包） | **`ota/install-fe`** | **否**（浏览器强刷） |
| **D. 控制面/探测面/plane** | `gr-service` `gr-probe-*` `gr-runtime` 等 | `gr-service-{ver}-*`（+ 必要时 cli） | **`ota/install-runtime`** + restart | **是** |
| **E. 面板 SPA** | `admin-ui` → `admin-spa` | `admin-spa.tgz` | 随 runtime 或单独面板/脚本装 SPA | 通常否 |
| **F. 全量产品 epoch** | 重大版本/新测试纪元 | 全套：service + 全部 so + fe + spa + manifest | `full-upgrade` 或分步 install-* | runtime 步需要 |

### 硬性禁止

1. **只改 analyze 却全量重编上传全部 so + 全站 install-all + 无必要 restart**。  
2. **能面板完成却默认 SSH curl 全量装**。  
3. **未改 plane 却 install-runtime**。  
4. **把 SSH 当「更快」的日常路径**。

### 推荐决策树

```
改了什么？
  ├─ 仅 analyze（mint/K/V）     → 编 sign 该 so → 发 GitHub → 面板 install analyze
  ├─ 仅 FE                       → 打 fe tgz → 面板 install-fe
  ├─ 仅 runtime/plane            → 编 service → 面板 install-runtime(restart)
  ├─ analyze + FE                → 两份资产 → 面板 install analyze + install-fe
  └─ 跨 plane+模块+FE 大版本     → 新 VERSION 全量 tag → 面板 full-upgrade
```

---

## 3. 版本号规则

| 场景 | VERSION |
|------|---------|
| 新测试纪元 / 用户要求「全新版本测」 | **抬 product VERSION**（如 6.0.8）并全量或按 F |
| 仅模块热修、同 epoch 可接受 | **可同 tag clobber 该 so**（manifest 更新 sha/sig），或 **小抬 patch** 后只发该模块 |
| FE-only 热修 | 优先 **抬 patch** 或 clobber `fe-{ver}.tgz`；`fe_impl`/product epoch 与 seal allowlist 一致 |

- 根目录 `VERSION` = Cargo workspace version = FE `fe/VERSION`（产品 SSOT）。  
- 模块 artifact `version` 字段与 manifest 一致，便于面板精确 install。

---

## 4. 面板主路径（标准操作）

控制面：`http(s)://…:28680/{console}/`（token 登录）

| 顺序 | API / UI | 说明 |
|------|----------|------|
| 1 | `POST ota/set-release-url` | `https://github.com/kullyeilert-jpg/gr-releases/releases/download/vX.Y.Z` |
| 2a | `POST ota/install` | `{"name":"analyze","version":"X.Y.Z","activate":true}` **增量** |
| 2b | `POST ota/install-fe` | 仅 FE |
| 2c | `POST ota/install-runtime` | `{"restart":true}` 仅 plane |
| 2d | `POST ota/install-all` | **仅当**本 tag 就是本轮完整模块集，或明确要同步全部模块 |
| 2e | `POST ota/full-upgrade` | 大版本一键（慎用） |
| 3 | health | control `/v1/health` + probe `/v1/health` + 抽样 session |

审计：所有 OTA 写 `audit_log`。

---

## 5. SSH 兜底（仅下列情况）

允许 SSH 的充分条件（满足一条即可，用完回到面板）：

1. 控制面进程 **无法响应** OTA API（core dump 循环、端口全死）  
2. **首次装机** / 无 admin token / 无公钥  
3. 面板 `install-runtime` **重启失败**且服务起不来  
4. 磁盘/权限/systemd/Nginx **基建**，非业务资产  
5. 用户 **明确要求** SSH 应急  

SSH 时仍优先：节点 `curl` GitHub 资产（与面板同源），避免构建机 scp 成为习惯。

端口：**21617**（非 22）。密钥见 `178.238.233.196/secrets.env`（不入库）。

---

## 6. 构建机增量发布约定

### 6.1 仅模块（例：analyze）

```bash
# 伪流程 — 实现见 scripts/release/build_signed_modules.sh 或后续 publish_modules.sh
VERSION=$(cat VERSION | tr -d '[:space:]')
export GR_RELEASE_VERSION=$VERSION
cargo build --release -p gr-module-analyze --features plugin
# sign → dist/release-$VERSION/libgr_analyze-${VERSION}-x86_64-linux-gnu.so
# 更新/合并 manifest.json 中 analyze 条目
gh release upload "v${VERSION}" \
  dist/release-$VERSION/libgr_analyze-*.so \
  dist/release-$VERSION/manifest.json \
  --repo kullyeilert-jpg/gr-releases --clobber
```

面板：`install` name=analyze only。

### 6.2 仅 FE

```bash
VERSION=$(cat VERSION | tr -d '[:space:]')
echo -n "$VERSION" > fe/VERSION
tar -C . -czf dist/release-$VERSION/fe-${VERSION}.tgz fe
gh release upload "v${VERSION}" dist/release-$VERSION/fe-${VERSION}.tgz --clobber
```

面板：`install-fe`。

### 6.3 全量（新 epoch）

`bash scripts/release/build_and_publish.sh` → 面板 `full-upgrade` 或分步。

---

## 7. 模块热更安全约束（实现已部分落地）

| 规则 | 原因 |
|------|------|
| 远程 so **ed25519 + sha256** | 生产 pubkey 必配 |
| stage **tmp + rename**，禁止原地覆盖已 dlopen 文件 | 防 SEGV |
| 默认 **仅 analyze live dlopen**；其它模块 activation marker（静态链接路径） | 防插件与静态冲突 |
| 同 path+version 已加载则 skip re-dlopen | 减 churn |

---

## 8. 验证清单（任何路径更新后）

- [ ] `GET control /v1/health`：version / mint_wire 符合预期  
- [ ] `GET probe /v1/health`：product_version 符合预期（FE epoch）  
- [ ] 仅模块热更：`restart_required=false`，analyze 时 `provider_active`  
- [ ] 涉及 B10：抽样 `probe_batches` 含 `B10_hw_curves`，无 422 风暴  
- [ ] 增量发布：GitHub release 资产列表 = 本轮变更，无多余全量 so  

---

## 9. 对 AI/自动化的记忆要点

1. **默认面板 OTA；SSH 仅兜底。**  
2. **按变更面增量打包、增量 install；禁止无脑全量。**  
3. analyze 算法优先 **单 so + 面板 install**。  
4. plane 才 **install-runtime**。  
5. FE 才 **install-fe**。  
6. 发版前写清：变更类型 A–F、资产列表、面板 API 序列。  
7. 用户要求「新版本全新测」→ 抬 VERSION + 按需全量 F；否则 patch 增量。

---

## 10. 修订

| 日期 | 说明 |
|------|------|
| 2026-08-11 | 初版：面板主路径 + 增量 so + SSH 兜底，按用户要求固化 |
