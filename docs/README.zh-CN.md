# greenpng — 项目指南（中文）

## 1. 项目定位

greenpng (GR) 通过在**真实访客的浏览器**内进行探测来判定会话是否真人，
而不是仅靠流量侧特征。

端到端管线：

1. **探测（浏览器内）** — 签名前端加载器在你的页面上运行，按会话从
   **多数据源**（输入动力学、设备栈、环境真实性、自动化痕迹）进行
   **多批次**分阶段采集。
2. **上传** — 浏览器把每个批次经密封上报管线提交到服务器；重放与篡改
   在入库前即被拒绝。
3. **分析（服务器端）** — 分析面对每个会话产出判定
   （`human | watch | bot`，附各轴置信度）与稳定、防碰撞的设备身份。
4. **返回** — 你的后端通过结果 API 取回判定（六语言 SDK；
   `public | sdk | diagnostic` 投影控制各调用方可见面）。

服务器端默认以单宿主二进制运行（内置探测面），并支持**多节点负载均衡
部署**：LB 模块把探测流量分发到共享数据层（PostgreSQL + Redis）前的多个
节点，采集与分析可水平扩展。

## 2. 仓库目录与架构

| 目录 | 内容 |
|---|---|
| `crates/` | Rust 工作区 — `gr-service`（控制面 + 管理控制台 + 内置探测面）、`gr-probe-core`、`gr-probe-plane`、`gr-probe-store`、`gr-ota`、`gr-admin`、`gr-runtime` 等 |
| `modules/` | 签名热更新模块源码（identity / brain / analyze / ingest / edge / probe_assets） |
| `probe/` | 浏览器探测前端（加载器、包链、密封上报客户端） |
| `panel/` | 管理面板 — Vue 源码（`admin-ui/`）+ 构建产物（`admin-spa/`），英文 + 中文 |
| `sdk/` | 六语言后端接入 SDK（仅结果拉取） |
| `spec/` | 运行时加载的线协议/评分规格 |
| `fixtures/` | 契约测试数据 |
| `scripts/` | 构建脚本与前端工具（`scripts/fe/checks/`） |
| `vendor/` | vendored 依赖源（pingora） |
| `install/` | 安装器、数据层 compose、升级脚本 |
| `release/` | 打包脚本（多架构构建、SBOM、模块签名） |
| `docs/` | 本指南，每语言一个文件 |

```
        访客浏览器
              │  <script src="/gr.js">（钉住，no-store）
              ▼
   FE 加载器 ──► /v1/sdk/bootstrap ──► 版本化包清单
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   包采集器（输入 · 设备 · 环境，多批次）
              │  密封提交（直连绑定 gv 域，TLS）
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service（控制面）          │
 │  网关 · 上报 · ops         │◄──┤  管理控制台 · 站点配置         │
 └────────────┬─────────────┘   │  模块注册表（OTA，签名）       │
              ▼                 └──────────────┬───────────────┘
   PostgreSQL（多节点时 + Redis）               │
              ▼                                │
   GET /v1/session/{id}/result ──► 商户 SDK（六语言）

   多节点: LB 模块把探测流量分发到共享数据层前的多个 gr-service 节点
```

关键组件：`gr-service` 是控制面（随机路径管理控制台、站点/配置管理、
签名 OTA 模块注册表）并内置探测面；`gr-probe-plane` 是 Pingora 网关
（密封上报与会话/结果 API）；浏览器前端把所有资产解析为版本不可变 URL，
缓存不可能跨版本供出旧探针；PostgreSQL（多节点时加 Redis）存储会话、
批次与分析结果。

## 3. 安装与使用

### 3.1 安装

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# 或指定版本 / 架构:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# 数据层容器化（仅 PostgreSQL/Redis; 服务本体保持宿主二进制）:
bash install/install.sh --version <VERSION> --with-docker --yes
```

安装器校验 sha256 + ELF + ed25519 模块签名，安装到 `/opt/greenpng`，
写入 `.env` 与 systemd 单元，装填并激活六个签名模块，并以控制/探测面
`/v1/health` 作门禁。首次登录凭据与随机控制台路径写在
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`。

### 3.2 更新

| 优先级 | 通道 | 适用 |
|---|---|---|
| P0 | 面板 OTA（set-release-url → install / install-fe / install-runtime） | 默认 |
| P1 | `install/release/update_runtime_from_github.sh`、`update_module_from_github.sh` | 无面板 / 免费节点 |
| P2 | SSH 手工 | 进程死亡 / 首装 |

所有更新都从本仓库拉取同一 tag 的签名 Release 资产。
Docker 是运行时容器，不是更新通道。

### 3.3 使用

1. 在管理面板**创建站点**：站点 ID、根域名，以及想随判定携带的业务字段
   Cookie 白名单（`password`/`token` 等敏感名称在服务端被强制拦截）。
2. **部署探针** — 三种模式：
   - *Nginx 第一方（推荐）*：`/gr.js` + `/gr/dist/v/` 反代到 pv 域、
     `/gr/v1/` 反代到 gv 域（Cookie 透传），向 HTML 注入
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>`。
   - *Cloudflare Worker*：注入同一标签并在源站代理 `/gr`。
   - *应用嵌入*：从 pv/CDN 直接加载加载器，`data-endpoint` 指向 gv/pv。
   浏览器上传**始终直连绑定 gv 域走 TLS**。
3. **接收结果** — 在面板创建站点 SDK key，然后轮询：

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **冒烟**（curl）：

```bash
# 携带白名单 cookie 打开会话
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# （真实 FE 批次或模拟批次之后）查结果:
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
