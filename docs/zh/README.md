# greenpng — 产品指南（中文）

浏览器端真人验证与反机器人智能：在真实访客浏览器中运行的签名探针、密封
上报管线、以及通过六语言 SDK 返回逐会话判定结果的分析面。

> 本指南提供 12 种语言版本 —— 见仓库根目录的
> [语言索引](../../README.md#documentation)。

## 1. greenpng 做什么

每个访客会话都会获得端上 + 服务端两侧评分：

- **真人/机器人判定** —— `human | watch | bot`，附各轴向置信度
  （输入动力学、设备栈一致性、环境真实性、自动化痕迹、历史复用）。
- **稳定设备身份** —— 防碰撞设备 ID，跨会话稳定，不依赖第三方 Cookie。
- **业务字段采集** —— 你在白名单里声明的 Cookie 字段
  （`user_id`、`plan_tier`…）在会话开启时采集并随判定返回；敏感名称
  （password/token…）在服务端被强制拦截。
- **IP 智能** —— 访客 IP 在入库最早点即掩码为 /24（IPv4）或 /48
  （IPv6）；可选富化把子网映射为 ASN/国家/城市（DB-IP 内置 MMDB、
  IPinfo、MaxMind 或自定义 HTTP 富化器）。密钥永远留在服务端。
- **结果取回 API** —— 商户用按站点的 SDK key 轮询判定结果；
  `public | sdk | diagnostic` 投影控制暴露面。

## 2. 核心特性

| 特性 | 价值 |
|---|---|
| 签名前端探针 | 防篡改浏览器包（ed25519），版本化不可变资产 URL `/dist/v/<ver>/g/<gen>/…` —— CDN 安全，杜绝缓存投毒 |
| 密封上报 | 包提交经密封；重放/篡改在上游即被拒 |
| 分析模块（OTA） | identity / brain / analyze / ingest / edge / probe_assets 以签名模块热更新，不停机 |
| 管理面板 | 随机路径独立控制台，单管理员 scrypt，审计日志，站点/策略/集成/保留/DSAR 管理，英文 + 中文 |
| 默认隐私 | IP 最早点掩码、Cookie 白名单、DSAR 导出/擦除、保留期清理 |
| 六语言 SDK | JS / Python / Go / PHP / Shell / Rust 的 `wait_for_result` 客户端 |
| 多节点就绪 | 内置 LB 模块、集群心跳、OTA 镜像 |

## 3. 架构

```
            访客浏览器
                  │  <script src="/gr.js">（钉住，no-store）
                  ▼
        FE 加载器 ──► /v1/sdk/bootstrap ──► 版本化包清单
                  │        (asset_base /dist/v/<fe>/g/<gen>/)
                  ▼
        包采集器（输入 / 设备 / 环境）
                  │  密封提交
                  ▼
   ┌──────────────┴───────────────┐
   │ gr-probe-plane (Pingora)      │  网关 / 上报 / 会话 API
   │  ├─ B8 网关（TLS SNI）        │  绑定域名直连上传
   │  ├─ 密封上报 + 批处理         │
   │  └─ /v1/ops/*（token 门禁）   │
   └──────────────┬───────────────┘
                  ▼
        gr-service（控制面）
          ├─ 管理控制台 + 管理 API（axum）
          ├─ 站点 / 策略 / 集成配置
          ├─ 启动修复：面板策略落盘 + 站点补偿同步
          └─ 模块注册表（OTA，签名）
                  ▼
        PostgreSQL（sessions、probe_batches、analysis_latest、admin）
                  ▼
        GET /v1/session/{id}/result ──► 商户 SDK（六语言）
```

**部署形态**

- **gr-service** —— 单进程：管理控制台 + 控制 API + 内置探测面。
  28680（控制台，随机路径）+ 28765（探测面回环）。
- **业务站 nginx** —— 服 `/gr.js` 并（可选）代理同源 API 前缀；浏览器
  上传必须走绑定 GV 域名的 Pingora TLS 直连。
- **数据库** —— 控制面与探测面用 PostgreSQL（实验环境有 SQLite 骨架）。

## 4. 快速安装

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # 版本见 VERSION 文件 / Releases
# 或带 docker 化数据层：
bash install.sh --version <VERSION> --with-docker --yes
```

安装器会校验 sha256 + ELF + ed25519 模块签名，安装到 `/opt/greenpng`，
写 `.env`，暂存并激活六个签名模块，启用 systemd 单元，并以
`/v1/health` 作健康门。

首次登录：一次性凭据写在
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` —— 随机控制台路径
（`/c-<hex>/`）也记录在该文件。没有 `/admin` 或 `/console/` 前缀，
密码登录是面板唯一入口。

## 5. 使用教程

### 5.1 创建站点

面板 **Sites → Create site**：

- `site_id` —— 你的租户 ID，嵌入代码中使用
- `root_domains` —— www 主机名（CORS + 主机绑定）
- `cookie_fields` —— Cookie 白名单，例如
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

站点保存时从 `control.sites` 流转到 `public.sites`（探测面），
gr-service 启动时还会对存量站做补偿同步。

### 5.2 部署探针（三种模式）

**A. Nginx 同源（推荐）** —— 业务站 vhost 代理引导加载器与同源 API
前缀：

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare Worker** —— worker 注入引导脚本并同源代理 `/gr`。
`/gr` 路径不要开"Under Attack"模式（挑战页会打断探针）。

**C. 站点脚本 / CDN 嵌入** —— 直接从 PV 加载引导 JS，`data-endpoint`
指向 GV。

所有模式下，加载器都通过 SDK bootstrap 解析包，且只取版本化不可变
URL。

### 5.3 接收结果（六语言 SDK）

在面板 SDK 页为站点创建 **backend key**。SDK 从不转发探针，只查询
结果：

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <站点 backend key>
```

| 语言 | 入口 |
|---|---|
| JS | `sdk/src/index.js` —— `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` —— `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` —— `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` —— `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` —— `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` —— `Client::new(base_url, key).wait_for_result(...)` |

站点 key 有租户边界：为 A 站铸的 key 读不了 B 站会话
（`403 sdk key site mismatch`），撤销立即失效（`401`）。

### 5.4 端到端冒烟（curl）

```bash
OPEN=$(curl -fsS -X POST https://gv.example.com/v1/session/open \
  -H 'Content-Type: application/json' \
  -H 'Cookie: user_id=u9; plan_tier=pro' \
  -d '{"site_id":"mysite","visitor_terminal_id":"vt_demo1"}')
SID=$(printf '%s' "$OPEN" | jq -r .session_id)

curl -fsS -X POST "https://gv.example.com/v1/session/$SID/analyze" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" -H 'Content-Type: application/json' -d '{}'

curl -fsS "https://gv.example.com/v1/session/$SID/result?projection=sdk" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" | jq '.sdk_projection'
```

### 5.5 保持更新

| 通道 | 命令 |
|---|---|
| 面板 OTA（默认） | 管理面板 → Modules / Runtime install |
| 更新脚本 | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| 手工 | SSH + 旧运行时回滚（`bin/releases/<v>` 保留） |

所有更新都拉取本仓同一 tag 的签名 Release 资产。

## 6. 仓库目录

| 目录 | 内容 |
|---|---|
| `crates/` | Rust 工作区 —— gr-service、gr-probe-core、gr-probe-plane、gr-probe-store、gr-ota、gr-admin、gr-runtime… |
| `modules/` | 签名热更模块源（identity / brain / analyze / ingest / edge / probe_assets） |
| `probe/` | 浏览器探针 FE（加载器、包、密封上报） |
| `panel/` | 管理面板 —— Vue 源码（`admin-ui/`）+ 构建产物（`admin-spa/`） |
| `sdk/` | 结果取回 SDK（JS / Python / Go / PHP / Shell / Rust） |
| `spec/` | 运行时加载的规格（bot 权重、目录） |
| `install/` | 安装器 + 更新器 + systemd 材料 |
| `release/` | 打包脚本（多架构 bundle、SLSA 证明） |
| `scripts/` | FE 契约检查与辅助脚本 |
| `docs/` | 本指南的 12 语言版本 |
| `VERSION` | 发版版本唯一事实源 |

## 7. 链接

- 发版与安装入口：本仓库
- 官网：https://www.greenpng.cc（产品介绍，英文 + 中文）
- 面板语言：English + 中文（`panel/admin-ui/src/i18n/` 内保持同步）
