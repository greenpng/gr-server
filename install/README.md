# greenpng 安装材料（→ gr-server 仓）

本目录是 greenpng 安装/升级材料的**源**（开发仓 `greenpng` 的 04 区），
经 `04-release-github-ci/release/sync_public_gr_server.sh` 整理进
发行仓 `greenpng/gr-server`（本地暂存仓 `greenpng/gr-server` → 版本分支 → 推送）。
用户从 **gr-server 仓** 获取安装器：

```bash
curl -fsSL -O https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
```

旧公共安装仓 `greenpng/install` 已退役；本目录 `.github/` 旧仓 CI 镜像已删除，
gr-server 仓的 CI 由根 `.github/workflows/`（gate-a / release）提供。

## 命名（greenpng 1.0.0 线，决裂原则）

- 安装前缀 `/opt/greenpng`；systemd 单元 `greenpng.service`；系统用户 `greenpng`。
- 数据层：PG 用户/主库 `greenpng`（compose 默认库），附加库 `gr_biz` / `gr_admin` / `gr_assoc`。
- 环境变量仅 `GR_*`（`GV*_*` 别名层已删除；greenpng 1.0.0 与旧 8.x 线不互通，升级需重装）。
- 版本线从 `1.0.0` 重新起算（`X.Y.Z`）。

## 目录结构

```
install/
├── install.sh                 # 用户一键安装 (sh)
├── ota_ed25519.pk             # 模块签名公钥 (ed25519, 公开)
├── release/
│   ├── update_runtime_from_github.sh  # 服务端运行时升级 (sha256 校验/失败回滚)
│   ├── auto_upgrade_check.sh          # 无人值守冷件门+调度 (flock/挂起/单调/窗口 → 调 updater)
│   └── update_module_from_github.sh   # 单模块热更 (链式验签)
├── docker/
│   ├── docker-compose.yml     # postgres(4库)+redis, 仅回环监听
│   ├── init-databases.sh      # 自动创建 gr_biz/gr_admin/gr_assoc
│   └── .env.example
├── systemd/
│   ├── greenpng.service       # systemd 单元模板 ({PREFIX} 占位)
│   ├── greenpng-auto-upgrade.service  # 无人值守冷件 oneshot (root, {PREFIX} 占位)
│   └── greenpng-auto-upgrade.timer    # 每晚 04:30+30m 抖动 (Persistent)
└── test/
    ├── smoke_install.sh       # 安装后冒烟 (控制面/管理面/探测面/业务会话)
    └── upgrade_rollback_check.sh  # 升级/版本控制契约 (本地 fixture, runner 用)
```

## 用户安装

### 1) 仅编排服务并填写连接信息（sh 安装）

```bash
bash install.sh --version 1.0.0
```

交互式向导按分组填写：

| 分组 | 变量 | 用途 |
|---|---|---|
| 业务面 | `GR_DATABASE_URL` | 会话 / 探针冷存储 / 分析队列 |
| 业务面 | `GR_BIZ_DATABASE_URL` | 业务看板 / 站点配置 |
| 业务面 | `GR_ASSOCIATION_DATABASE_URL` | 关联分析 |
| 管理面 | `GR_ADMIN_DATABASE_URL` | 管理面板 / 审计 / 集群 OTA |
| 缓存 | `GR_REDIS_URL` | soft store / 多 worker 共享 L2（可选，留空=本地文件） |
| 登录 | `GR_OFFICIAL_URL` / `GR_OAUTH_ADMIN_EMAILS` | 管理面板 OAuth 跳转官网域名 + 管理员邮箱白名单 |

非交互（脚本化 / CI）：`GR_*` 变量写入 my.env 后
`bash install.sh --version 1.0.0 --env-file my.env --no-systemd`。

### 2) 数据服务也由安装器管理（docker 方式）

```bash
bash install.sh --version 1.0.0 --with-docker
```

自动在本机起 `postgres(4库)+redis` 容器（`docker/.env` 随机密码，幂等复用），
预填连接串后继续走 sh 安装流程。

其它常用参数：`--arch auto|x86_64|aarch64`、`--prefix /opt/greenpng`、`--yes`、
`--no-systemd`（无 systemd 环境，后台拉起+日志在 `$PREFIX/log/`）。

### 3) 安装后冒烟

```bash
bash test/smoke_install.sh --prefix /opt/greenpng
```

断言：控制面 `/v1/health` 200、管理面板未认证 `{console}/api/me` 401、
探测面 `/v1/health` 200、业务会话 `/v1/session/open` 成功。

### 4) 运行时升级

```bash
VERSION=1.0.1 bash release/update_runtime_from_github.sh   # 在安装机上执行
```

- 从 gr-server release 拉 `manifest-index.json` → 本架构整包 `greenpng-<ver>-<arch>.tar.gz`
  （index 钉 sha256）→ 安全解包 → 验包内 manifest 根签名；
- 运行时/CLI/FE 全部取自包内并按 manifest `sha256` 校验（fe_tree 逐文件）；
- 装入 `bin/releases/<ver>/` 版本化 slot，写入 `VERSION` / `fe/VERSION`；
- systemd 重启 + 健康门，失败自动回滚旧二进制（上次成功备份）。

### 5) 无人值守自动升级（可选，`GR_AUTO_UPGRADE=1`）

安装时传 `GR_AUTO_UPGRADE=1`（或向导选 1）即装并启用
`greenpng-auto-upgrade.timer`；脚本始终装到 `$PREFIX/sbin/`（root 属主）。
生效还需两步：面板"升级所选"勾 **自动应用**（可填维护窗口）；服务 env 加
`GR_AUTO_OTA_HOT=1` 启用热件（模块/FE）自动跟进——默认 `warn` 只记日志不动作。

- 门序（`release/auto_upgrade_check.sh`，全过才调 updater）：flock →
  hold 挂起（面板暂停/回滚锁存，24h 冷却）→ `desired.auto_apply` →
  严格 semver 单调（只升不降）→ 维护窗口；
- 每次决策与结果在 `journalctl -u greenpng-auto-upgrade` 与
  `data/auto_upgrade_state.json`（面板"自动升级"卡展示）；
- 热层（服务内线程）升级-only 且模块激活带单调地板；失败指数退避（20s→300s）；
- 完整设计与 runbook：`docs/iss/ota-unattended-auto-upgrade-design.md`、
  `docs/guides/11-OPERATIONS.md`；门契约沙测
  `03-local-test-lab/tests/local/auto_upgrade_gate_sandbox.sh`。

## 安全说明

- 整包按 `manifest-index.json` 的 `bundle_sha256` 校验后解包；包内 manifest 由根钥
  验签，runtime/cli/modules/fe_tree/admin_tree 逐项 `sha256` 校验（缺失即失败）。
- 模块 `.so` 由运行时 `gr-cli stage` 做**链式验签**后 `activate`：根公钥验 `sig`
  （旧发布兼容），绑定了 release key 的发布还须通过 `sig2`（发布密钥）与证书链
  （`verify_manifest_chain` / `verify_artifact_sig_chain`）；公钥为公开产物，
  根私钥与每版本 release 私钥仅存于打包端/CI secret，release 私钥不随资产发布。
- 服务启动时对 active 模块做完整性自检（链签 + sha256，prod 或
  `GR_STRICT_BOOT_VERIFY=1` 下失败即拒绝启动）。
- tar 解包拒绝绝对路径/`..`/符号链接（R-05）。
- 默认仅回环监听（`.env` 中 `GR_BIND` 等 127.0.0.1），对外暴露需显式修改。
