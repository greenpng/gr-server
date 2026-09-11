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

- **模块与 FE 热更新** — 面板 `install` / `install-fe` 无需重启进程即换；
  浏览器经版本不可变、内容哈希的资源 URL 自动取新 FE。
- **runtime 二进制需重启** — `install-runtime` 换二进制、重启服务，并把
  **版本戳门禁在健康探针上**；`HEALTH_FAIL` 自动还原旧二进制、FE/spec 树
  与 `VERSION`（不会留下半升级、自动升级锁死的主机）。
- **自动升级**（面板 `cluster-apply` 选择性开启）：目标版本 + 生效窗口
  （默认 `04:00-05:30`），节点走同一健康门流程。
- **内存注意（长运行主机）**：安装器会在 `/opt/greenpng/.env` 写入
  `MALLOC_ARENA_MAX=4` —— glibc 多线程并发分配时新开的 64MB malloc
  arena 会在其线程消亡后继续驻留并保持高水位，否则表现为 RSS 慢棘轮
  （6 核节点实测 1 小时 ~1.5GB）。存量安装手工追加该行并重启即可，
  RSS 回落到工作集水位（实测 ~100-200MB）。详见根 README §3。

### 3.3 卸载

```bash
bash install/uninstall.sh          # 标准移除
bash install/uninstall.sh --purge  # 连 greenpng 系统账户一并移除
```

停用并移除三个 systemd 单元、polkit 自 OTA 规则、`/opt/greenpng` 安装树、
运行时锁与 OTA 缓存（`--purge` 另加系统账户）。**Docker 数据服务与数据库
卷刻意不动**——确要清数据时才用
`docker compose -f install/data-compose.yml down -v`。

### 3.4 使用

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

## 4. 管理面板参数

以下参数都在面板 **Config（调参）页**编辑（保存 → 发布）：**发布节点立即
生效，集群节点 ≤30s，无需重启。** 每个字段带一行提示（中/英），此处为
运维向摘要。

**速率限制**（经 PostgreSQL 共享窗口，集群级一致）：

| 参数 | 默认 | 含义 |
|---|---|---|
| `rate_limit_open_per_min` | **0 = 不限** | 每站每分钟会话开启总数 |
| `rate_limit_ingest_per_min` | **0 = 不限** | 每站每分钟探测批次上传总数 |
| `rate_limit_analyze_per_min` | **0 = 不限** | 每站每分钟直接分析调用总数 |
| `rate_limit_complete_per_min` | **0 = 不限** | 每站每分钟完成回执总数 |
| `rate_limit_result_per_min` | **0 = 不限** | 每站每分钟结果读取总数 |
| `rate_limit_client_event_per_min` | **0 = 不限** | 每站每分钟 FE 遥测事件总数 |
| `rate_limit_client_event_per_ip_per_min` | **100** | **单个 IP** 每分钟 FE 遥测事件数——超限只掐该 IP，其他访客不受影响；0 = 关 |

v1.0.14 策略：站点总量默认**关**（总量一刀切会把混在机器人洪峰里的真人
一起掐掉）；遥测通道改为**按 IP** 限。429 响应体标明触发的层
（`client_event:ip` / `client_event:site`）。

**站点在 CDN 后面时**，按 IP 限的桶键是服务端实际看到的 IP——若前置
代理不做真实 IP 还原，那就是 **CDN 边缘 IP**。服务端只信任来自受信
代理对端（默认 loopback，`GR_TRUSTED_PROXIES` 可扩展）的
`X-Real-IP`/`X-Forwarded-For`，绝不直接信任客户端自报头。nginx 前置时
用 `realip` 模块还原访客 IP（网段取 CDN 公布列表，Cloudflare 用
`CF-Connecting-IP` 头，其他 CDN 用 `True-Client-IP`），按 IP 限流与
遥测归属即落在真实访客上。只有来自所列网段的连接才会采信该头——
绕过 CDN 直连源站伪造头仍按其自身 IP 记账，防伪造。实测案例见根
README §4.1（2026-09-11 生产验证）。

**洪水加固**：`robot_fastlane_enabled`（开）、`hot_max_vts`（8192）、
`arm_sweep_interval_ms`（15000）、`arm_sweep_cap`（256）、
`analyze_claim_batch_flood`（16）。

**周期与热冷分层**：`cycle_cool_ms`（86400000）、`cycle_incomplete_ms`
（259200000）、`session_inactivity_ms`、`session_hard_max_ms`、
`hot_idle_ms`、`cold_ttl_ms`（604800000）、`cold_promote_window_ms`
（86400000）、`cold_purge_interval_ms`（300000）、
`complete_on_commercial_silicon`（开）。

**分析触发与 FE 上传**：`analyze_idle_upload_ms`（20000）、
`analyze_debounce_ms`（40）、`return_identity_idle_ms`（45000）、
`rpa_idle_analyze_ms`（25000）、`hard_max_attempts`（8）/`soft_max_attempts`
（4）/`deepen_max_attempts`（6）/`rpa_max_attempts`（3）、`fail_budget_n`
（24）/`fail_budget_window_ms`（90000）、`hard_sla_retries`（5，基础延迟
2000ms）、`upload_concurrency`（6）→ `upload_mid_ramp`（12）于
`upload_ramp_after`（14）之后、`upload_max_retries`（5）、
`client_alive_retry_ms`（30000）、`multi_tick_max`（96）、
`empty_kick_patience`（20）。

**并发 / 保留 / 站点**：

| 区域 | 默认 | 生效 |
|---|---|---|
| 并发 workers | analyze / ingest / gateway 数量 | analyze 热缩放立即；其余走集群 desired（≤30s） |
| 数据保留 | analysis 30天 · session 30天 · velocity 7天 · ops 14天 · master 90天 · cold_ttl 7天 · 批量 200 · 间隔 300s | 下个清理周期；手动清理立即（有界批次） |
| 站点 | 采集开关、策略、域名/SSL、SDK key | 采集/策略下一请求即效；SSL 热（SNI）；key 即刻可用 |

## 5. 日志与可观测

日志走 journald（`journalctl -u greenpng.service`），默认 INFO、安静设计：

| 日志线 | 含义 |
|---|---|
| `ingest_ack … durability_state=stored_durable` | 每个被接受的探测批次（来源 + 批次类型） |
| `analyze claimed n=… worker=…` | 调度领取（每 worker 5s 节流） |
| `auto-analyze complete sid=… rev=… eval_ms=…` | 判定产出 + 评估耗时 |
| `arm sweep armed=…` | 热→冷扫挂臂数（空扫不记） |
| `cold purge loop / retention purge loop started` | 启动时后台 TTL/保留清理所有权 |
| `http_4xx_5xx_30s n=… sample="…"` | 4xx/5xx 聚合窗（每 30s 一条 WARN，不逐请求刷屏） |
| `pg worker reconnected / admin pg reconnected / control admin pg reconnected` | 数据库重启后的 PG 自愈重连 |
| 队列超上限 | 超限期间每 60s 重复告警 |

**ops 事件流**（面板 结果/审计 页、`/v1/ops/events/export`）：
`ops_server_events`（密封接受、协议拒绝、限流）与 `ops_client_events`
（FE 诊断；服务端剥离密钥/原始序列，IP 存 /24），由 `ops_retention_days`
（14 天）约束增长。

**开关**：管理设置 `ops_client_events_enabled=0`（或
`GR_OPS_CLIENT_EVENTS=0`）整体关闭 FE 遥测上传；实验室形态可用
`GR_RATE_LIMIT_FORCE=1` / `GR_RATE_LIMIT_OFF=1` 强制限流开/关。管理动作
（登录、OTA、配置发布、key 签发）全部带 actor + detail 进面板审计页。

## 6. 使用的开源项目

**vendor 内置**：[Pingora](https://github.com/cloudflare/pingora)
（Apache-2.0，Cloudflare）——探测面服务层，位于 `vendor/pingora/`。

**主要 crates.io 依赖**：axum / tower-http / tokio（控制面 HTTP + 异步）、
postgres / rusqlite（存储）、ed25519-dalek / x25519-dalek / aes-gcm / hkdf /
hmac / scrypt（签名、密封、凭据）、sha2 / blake3（摘要）、reqwest-rustls
（OTA/webhook 拉取）、dashmap / arc-swap / parking_lot（共享态）、tracing
（日志）、serde / chrono / uuid / semver / regex / clap / sysinfo / flate2
（工具）。每版全量清单见 **`sbom.cdx.json`**（CycloneDX）。

**构建其上的基础设施**：PostgreSQL（数据层）、Redis（多节点）、nginx
（第一方加载模式）、systemd + polkit（生命周期 + 自 OTA 授权）、
Cloudflare CDN/Workers（可选前置）。设计参考：签名包仓库的信任链模型
（清单索引 → 逐资产签名 → 钉根公钥）与 Cloudflare Pingora 服务模型。
**未包含任何第三方探针/反爬代码**——`crates/` 与 `modules/` 中的探测、
评分与分析代码均为本项目原创。
