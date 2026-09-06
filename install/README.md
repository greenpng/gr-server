# green-v7 install（公共安装仓库）

面向用户的安装、发布与**安装包测试**仓库（`greenpng/install`）。**不含源码**
（交叉编译在私有 green-v7 完成），但**自带 CI**：安装/升级测试跑在本仓库的
GitHub runner 上——公共仓库标准 runner **免费**（Linux x86_64/aarch64），
不会消耗账号计费分钟。

- 这里保存：用户安装器、数据服务编排、**已编译的发布产物**（release）、
  消费端冒烟与升级脚本、以及跑在这些资产上的测试 workflow。
- 本目录 `install/` 是 green-v7 仓库内的同步镜像，向公开仓库推送使用
  `bash release/sync_public_install.sh`（见 green-v7 `docs/testing/03-RUNNER-TESTS.md`）。

## 发布与测试模型（在哪里构建、在哪里测）

1. **交叉编译（私有 green-v7，按需手动 dispatch）**：`release-multiarch.yml`
   在 x86_64 + aarch64 原生 runner 上编译、加固签名、生成 manifest/SBOM
   （每发布随机 `build_id` + 每版本临时签名密钥 + 模块双签 + 字符串混淆，
   详见 green-v7 `docs/guides/07-HARDENING.md`）。发布动作低频，私有分钟开销可忽略。
2. **发布（到这里）**：产物上传到本仓库 release（`manifest-index.json` 按架构
   索引；双架构各自 manifest + 单份 FE bundle 的完整性守卫）。历史 6.x/7.x
   与 8.x 版本均在同一仓库，8.0.0 起用户侧命名统一为 GR。
3. **测试（本仓库，免费公共 runner）**：`.github/workflows/` 直接消费本仓库
   release 资产（或本地 fixture）：
   - `test-install-sh.yml`：sh 安装（用户自备数据服务）x86_64/aarch64；
   - `test-install-docker.yml`：`install.sh --with-docker` 安装 x86_64/aarch64；
   - `test-install-upgrade.yml`：升级/版本控制（版本化 slot、VERSION/fe/VERSION、
     sha256 篡改拒绝、回滚保留）。

> 公共仓库的标准 runner 免费，因此重复度高的安装/升级测试放在这里；私有仓
> green-v7 的 workflow 只保留低频的源码级套件（多节点、面板 OTA、契约、业务、
> 打包），且仅在手动 dispatch / 周定时时运行，不再随 push/PR 自动触发。

## 目录结构

```
install/
├── install.sh                 # 用户一键安装 (sh)
├── ota_ed25519.pk             # 模块签名公钥 (ed25519, 公开)
├── release/
│   └── update_runtime_from_github.sh  # 服务端运行时升级 (sha256 校验/失败回滚)
├── docker/
│   ├── docker-compose.yml     # postgres(4库)+redis, 仅回环监听
│   ├── init-databases.sh      # 自动创建 gr_biz/gr_admin/gr_assoc（旧 gv6_* 兼容）
│   └── .env.example
├── systemd/
│   └── green-v7.service       # systemd 单元模板 ({PREFIX} 占位)
├── test/
│   ├── smoke_install.sh       # 安装后冒烟 (控制面/管理面/探测面/业务会话)
│   └── upgrade_rollback_check.sh  # 升级/版本控制契约 (本地 fixture, runner 用)
└── .github/workflows/
    ├── test-install-sh.yml        # runner: sh 安装 (x86_64+aarch64, 免费)
    ├── test-install-docker.yml    # runner: docker 安装 (x86_64+aarch64, 免费)
    └── test-install-upgrade.yml   # runner: 升级/版本控制 (x86_64+aarch64, 免费)
```

（打包/发布工具保留在私有 green-v7 `release/`，不随本仓库公开。）

## 用户安装

### 1) 仅编排服务并填写连接信息（sh 安装）

```bash
curl -fsSL -O https://raw.githubusercontent.com/greenpng/install/main/install.sh
bash install.sh --version 8.0.0
```

交互式向导按分组填写：

| 分组 | 变量 | 用途 |
|---|---|---|
| 业务面 | `GR_DATABASE_URL`（旧 `GV6_DATABASE_URL`） | 会话 / 探针冷存储 / 分析队列 |
| 业务面 | `GR_BIZ_DATABASE_URL`（旧 `GV6_BIZ_DATABASE_URL`） | 业务看板 / 站点配置 |
| 业务面 | `GR_ASSOCIATION_DATABASE_URL`（旧 `GV6_ASSOCIATION_DATABASE_URL`） | 关联分析 |
| 管理面 | `GR_ADMIN_DATABASE_URL`（旧 `GV6_ADMIN_DATABASE_URL`） | 管理面板 / 审计 / 集群 OTA |
| 缓存 | `GR_REDIS_URL`（旧 `GV5_REDIS_URL`） | soft store / 多 worker 共享 L2（可选，留空=本地文件） |
| 登录 | `GR_OFFICIAL_URL` / `GR_OAUTH_ADMIN_EMAILS`（旧 `GV6_*`） | 管理面板 OAuth 跳转官网域名 + 管理员邮箱白名单 |

非交互（脚本化 / CI）：

```bash
curl -fsSL -O https://raw.githubusercontent.com/greenpng/install/main/install.sh
GR_* 变量写入 my.env 后（旧 GV6_/GV5_ 文件仍兼容）:
bash install.sh --version 8.0.0 --env-file my.env --no-systemd
```

### 2) 数据服务也由安装器管理（docker 方式）

```bash
bash install.sh --version 8.0.0 --with-docker
```

自动在本机起 `postgres(4库)+redis` 容器（`docker/.env` 随机密码，幂等复用），
预填连接串后继续走 sh 安装流程。

其它常用参数：`--arch auto|x86_64|aarch64`、`--prefix /opt/green-v7`、`--yes`、
`--no-systemd`（无 systemd 环境，后台拉起+日志在 `$PREFIX/log/`）。

### 3) 安装后冒烟

```bash
bash test/smoke_install.sh --prefix /opt/green-v7
```

断言：控制面 `/v1/health` 200、管理面板未认证 `{console}/api/me` 401、
探测面 `/v1/health` 200、业务会话 `/v1/session/open` 成功。

### 4) 运行时升级

```bash
VERSION=8.0.0 bash release/update_runtime_from_github.sh   # 在安装机上执行
```

- 从本仓库 release 下载运行时/CLI/FE，按 manifest `sha256` 校验（缺失即失败）；
- 装入 `bin/releases/<ver>/` 版本化 slot，写入 `VERSION` / `fe/VERSION`；
- systemd 重启 + 健康门，失败自动回滚旧二进制（上次成功备份）。

## 测试（本仓库 runner，手动/打 tag 触发）

```text
Actions → test-install-sh / test-install-docker / test-install-upgrade → Run workflow
```

- 三个 workflow 均矩阵覆盖 x86_64（ubuntu-24.04）+ aarch64（ubuntu-24.04-arm）；
- 打 `v*` tag 自动跑；`test-install-sh/docker` 可传 `version`（留空 = 最新发布）；
- 冒烟用同一份 `test/smoke_install.sh`；升级测试用本地 fixture 发布树
  （`test/upgrade_rollback_check.sh`），覆盖版本化 slot、VERSION 同步、摘要
  篡改拒绝与回滚保留。

## 安全说明

- 运行时/FE 下载后按 `manifest` 的 `sha256` 校验（缺失即失败）。
- 模块 `.so` 由运行时 `gr-cli stage` 做**链式验签**后 `activate`：根公钥验 `sig`
  （旧发布兼容），绑定了 release key 的发布还须通过 `sig2`（发布密钥）与证书链
  （`verify_manifest_chain` / `verify_artifact_sig_chain`）；公钥为公开产物，
  根私钥与每版本 release 私钥仅存于打包端/CI secret，release 私钥不随资产发布。
- 服务启动时对 active 模块做完整性自检（链签 + sha256，prod 或
  `GR_STRICT_BOOT_VERIFY=1`（旧 `GV6_STRICT_BOOT_VERIFY=1`）下失败即拒绝启动）。
- tar 解包拒绝绝对路径/`..`/符号链接（R-05）。
- 默认仅回环监听（`.env` 中 `GR_BIND`/旧 `GV6_BIND` 等 127.0.0.1），对外暴露需显式修改。
