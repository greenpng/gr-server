# greenpng — 官方项目技术指南（中文版）

> **项目定位**：企业级浏览器端人机验证与实时反机器人智能分析平台。通过在真实访客浏览器内执行密码学签名的分阶段探针，经上游密封信封管道传输，为后端提供亚毫秒级风控判定与稳定设备身份。

---

## 📑 目录

- [1. 项目定位与核心优势](#1-项目定位与核心优势)
- [2. 系统架构与数据流向](#2-系统架构与数据流向)
- [3. 服务器安装（全新节点部署）](#3-服务器安装全新节点部署)
- [4. 服务器卸载（干净移除）](#4-服务器卸载干净移除)
- [5. 系统更新与 OTA 升级](#5-系统更新与-ota-升级)
- [6. 管理控制台与运行期参数调优](#6-管理控制台与运行期参数调优)
  - [6.1 访问随机控制台路径](#61-访问随机控制台路径)
  - [6.2 接口流控与限流参数](#62-接口流控与限流参数)
  - [6.3 CDN 真实客户端 IP 还原配置](#63-cdn-真实客户端-ip-还原配置)
  - [6.4 防洪水加固与爬虫快道 (Robot Fastlane)](#64-防洪水加固与爬虫快道-robot-fastlane)
  - [6.5 冷热数据存储分层与清理周期](#65-冷热数据存储分层与清理周期)
  - [6.6 分析触发条件与客户端重试阶梯](#66-分析触发条件与客户端重试阶梯)
- [7. 前端探针集成模式](#7-前端探针集成模式)
- [8. 多语言 SDK 接入示范](#8-多语言-sdk-接入示范)
- [9. 开源项目引用与致谢](#9-开源项目引用与致谢)
- [10. 安全与隐私合规承诺](#10-安全与隐私合规承诺)

---

## 1. 项目定位与核心优势

不同于单纯依赖 HTTP 请求头或流量指纹的传统风控方案，**greenpng** (GR) 专注于在**真实访客的浏览器 DOM 运行时环境**内进行深层多批次特征提取。

### 核心特性矩阵
1. **B0~B12 多批次递进式探测矩阵**：
   - `B0 引导环境`：系统基础特征与 JavaScript 原型链真实性校验。
   - `B1 冲突检测`：原型链污染、原生函数被恶意 Hook 篡改分析。
   - `B2 硬件画像`：GPU 芯片厂商/渲染器、CPU 并发核心数、屏幕物理色彩深度。
   - `B3 系统指纹`：操作系统内核、音频上下文（AudioContext）微熵、系统已安装字体。
   - `B8 网关一致性`：JA4 / TLS 握手特征与应用层 SNI 一致性交叉比对。
   - `B10 硬件曲线`：WebGL 着色器浮点微抖动计算与执行耗时拟合。
   - `B10x 晶振热漂移`：硅片晶振高精热频偏差与超低功耗微计算漂移探测。
   - `B12 反伪装反自动化`：Puppeteer、Playwright、Selenium 及各类无头浏览器专有特征捕获。
2. **高抗碰撞稳定设备 ID**：跨会话保持高度稳定的确定性设备指纹，完全脱离第三方 Cookie，无侵入式跟踪。
3. **原生隐私合规 (Privacy-by-Design)**：访客 IP 入库即自动掩码为 `/24` (IPv4) 或 `/48` (IPv6)；严密隔离个人身份信息（PII），天然满足欧盟 GDPR 与加州 CCPA 规范。
4. **服务端 Cookie 业务字段安全捕获**：在管理面板配置白名单后，服务端在会话建立阶段直接从 HTTP 请求头提取业务标识（如 `user_id`、`plan_tier`）并附加到判定结果，杜绝前端脚本泄露敏感数据。
5. **无感热更新 (OTA)**：基于 Ed25519 签名与不可变内容寻址，分析模型与前端探针文件支持毫秒级平滑热替换。

---

## 2. 系统架构与数据流向

```
        访客浏览器 (Visitor Browser)
              │  <script src="/gr.js">（钉住，no-store，CDN 友好）
              ▼
    FE 加载器 ──► /v1/sdk/bootstrap ──► 版本化探针资产清单
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
    分阶段采集矩阵（输入动力学 · 硬件 · 系统 · WebGL · 芯片漂移 · 自动化痕迹）
              │  密文信封（直连绑定 gv 域，TLS）
              ▼
  ┌───────────────────────────┐         ┌──────────────────────────────┐
  │  gr-probe-plane (Pingora) │         │ gr-service (控制平面)        │
  │   TLS 网关 · 密封上报接收  │◄────────┤  管理控制台（随机路径）      │
  │   B8 网关 TLS SNI 校验    │         │  签名 OTA 模块注册表         │
  └─────────────┬─────────────┘         └──────────────┬───────────────┘
                ▼                                      │
       分层混合存储 (PostgreSQL + Redis)               │
                ▼                                      │
  GET /v1/session/{id}/result ─────────────────────────┘
                ▼
     业务微服务后端（Go、Rust、TypeScript、Python、PHP、Shell SDK）
```

- **`gr-service`**：控制面中枢，承载随机路径管理控制台、多租户站点配置、审计日志与签名 OTA 模块分发。
- **`gr-probe-plane`**：基于 **Cloudflare Pingora** 构建的边缘数据面网关，负责高并发 TLS 卸载与密文探针包接收。
- **`gr-probe-core`**：核心风控与设备聚类算法库，负责计算设备指纹、关联图谱推理与多轴向置信度评分。
- **`gr-probe-store`**：分层存储系统，融合内存 L1 热区（抗瞬时并发与去重）与 PostgreSQL L3 关系型数据库。

---

## 3. 服务器安装（全新节点部署）

### 运行环境要求
- **操作系统**：Linux x86_64 或 aarch64（Ubuntu 20.04+、Debian 11+、RHEL/CentOS/Rocky 8+）。
- **初始化系统**：`systemd`（服务托管与自动更新守护必备）。
- **数据库支持**：现成的 PostgreSQL 13+，或使用 `--with-docker` 参数自动编排本地 PostgreSQL + Redis。
- **网络端口**：`28680`（管理控制台与控制面 API）、`28765`（探测面，Pingora 边缘网关：`/gr.js`、`/dist`、`/v1` 会话/上报/结果）、`28766`（Gateway，早期绑定端口，预留）。

### 一键自动化安装
```bash
# 通过官方签名校验安装脚本一键部署：
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash

# 或者显式指定版本与架构：
bash install/install.sh --version 1.0.14 --arch x86_64 --yes

# 携带 Docker 容器化数据层（自动拉起 PostgreSQL 和 Redis）：
bash install/install.sh --version 1.0.14 --with-docker --yes

# 演练模式（仅校验签名链与清单，不修改本地系统）：
bash install/install.sh --dry-run
```

### 安装脚本执行步骤说明：
1. **清单与数字签名链校验**：从官方 Releases 解析版本包，校验内置的 Ed25519 根密钥指纹、`manifest-index.json` 的 SHA-256 摘要与二进制 ELF 格式。
2. **部署目录树**：安装至 `/opt/greenpng`（`bin/`、`fe/`、`spec/`、`.env` 等）。
3. **安全隔离配置**：创建独立的系统账户与用户组 `greenpng:greenpng`，配置 `/etc/polkit-1/rules.d/49-greenpng-self-ota.rules` 提权规则。
4. **服务注册与启动**：注册并拉起 `greenpng.service` 与定时更新器 `greenpng-auto-upgrade.{service,timer}`。
5. **健康检查门禁**：循环探测 `/v1/health` 直至返回 HTTP 200。
6. **输出初始引导凭据**：在控制台随机路径生成首次登录账号密码，存放在：
   ```bash
   cat /opt/greenpng/data/admin/admin_bootstrap_once.txt
   ```

---

## 4. 服务器卸载（干净移除）

greenpng 提供官方标准卸载脚本 `install/uninstall.sh`，可安全、彻底地清理所有注册的系统级服务与残留文件：

```bash
# 1. 标准卸载（停止服务、删除 /opt/greenpng、清理 polkit 规则与定时器）：
sudo bash install/uninstall.sh

# 2. 深度清理模式（追加 --purge，连同系统专属账号 greenpng 一并删除）：
sudo bash install/uninstall.sh --purge

# 3. 指定自定义安装路径：
sudo bash install/uninstall.sh --prefix /opt/greenpng --purge
```

### 卸载脚本处理清单：
- **systemd 服务注销**：停止并注销 `greenpng.service`、`greenpng-auto-upgrade.timer`、`greenpng-auto-upgrade.service`，并执行 `systemctl daemon-reload`。
- **安全提权规则清除**：彻底删除 `/etc/polkit-1/rules.d/49-greenpng-self-ota.rules`，消除系统提权隐患。
- **程序目录清理**：彻底清空删除 `/opt/greenpng`（程序本体、前端静态文件、规则库与运行日志）。
- **锁与临时文件清理**：清除 `/run/greenpng-auto-upgrade.lock` 与 `/tmp/gr-ota-*` 临时包。
- **安全免死保证**：PostgreSQL 业务数据与外部 Docker 存储卷**绝不主动触碰**，保障数据绝对安全。

---

## 5. 系统更新与 OTA 升级

| 优先级 | 升级通道 | 触发方式与目标 | 停机时间 |
|:---:|:---|:---|:---:|
| **P0** | **管理控制台一键 OTA** | 在管理后台点击 `Set Release URL` → `Install Runtime / FE` | 模块/前端零停机；二进制 < 1秒重启 |
| **P1** | **CLI 命令行更新脚本** | 执行 `sudo bash /opt/greenpng/install/release/update_runtime_from_github.sh` | < 1秒服务自愈重启 |
| **P2** | **无人值守自动升级** | `greenpng-auto-upgrade.timer` 在预设窗口期内静默比对升级 | 自动化静默执行 |

### 原子切换与健康门禁自动回滚
在更新主运行时二进制时：
1. 更新器暂存新版本并建立备份快照（`gr-service.bak.<timestamp>`）。
2. 服务重启并针对 `/v1/health` 连续进行 8 轮健康探测。
3. 仅当健康检查通过后，才正式更新 `/opt/greenpng/VERSION`。
4. 若新版本启动失败（`HEALTH_FAIL`），系统立即执行**全量原子回滚**，自动恢复旧版本二进制、前端资源并退出告警，避免生产节点死锁。

---

## 6. 管理控制台与运行期参数调优

### 6.1 访问随机控制台路径
为防止全网自动化脚本扫盘和撞库攻击，控制台默认部署在随机生成的 URL 路径下：
```
http://<服务器IP>:28680/<随机控制台路径>/
```
*(路径与密码可从 `/opt/greenpng/data/admin/admin_bootstrap_once.txt` 读取)*。

### 6.2 接口流控与限流参数
可在管理控制台的 **Config** 页面热修改并发布，集群各节点在 $\le 30$ 秒内自动生效，无需重启：

| 配置参数 | 默认值 | 说明 |
|---|:---:|---|
| `rate_limit_open_per_min` | `0` (不限) | 单站点每分钟最大允许的会话建立数 |
| `rate_limit_ingest_per_min` | `0` (不限) | 单站点每分钟最大允许的探针批次上报数 |
| `rate_limit_analyze_per_min` | `0` (不限) | 单站点每分钟最大直接分析计算数 |
| `rate_limit_result_per_min` | `0` (不限) | 单站点每分钟最大结果查询请求数 |
| `rate_limit_client_event_per_min` | `0` (不限) | 单站点每分钟客户端遥测事件总量 |
| `rate_limit_client_event_per_ip_per_min` | `100` | **单 IP** 每分钟客户端事件上限（防单点刷量） |

### 6.3 CDN 真实客户端 IP 还原配置
当 greenpng 部署在 Cloudflare 或 Nginx 代理后方时，需配置受信任代理以获取真实访客 IP：

```nginx
# /etc/nginx/conf.d/realip.conf
set_real_ip_from 173.245.48.0/20;   # Cloudflare 官方 IPv4 网段
set_real_ip_from 103.21.244.0/22;
set_real_ip_from 2400:cb00::/32;    # Cloudflare 官方 IPv6 网段
real_ip_header CF-Connecting-IP;
```

### 6.4 防洪水加固与爬虫快道 (Robot Fastlane)
- `robot_fastlane_enabled` (`true`)：主流搜索引擎公开爬虫（如 Googlebot）自动走快速通道，跳过繁重的深层计算与存储，直接返回预置判定。
- `hot_max_vts` (`8192`)：内存热区最大保留的访客终端数，超出后走 LRU 降级。
- `arm_sweep_interval_ms` (`15000`)：空闲会话定时扫描周期。
- `arm_sweep_cap` (`256`)：每次扫描允许批量推进的最大任务数，避免大锁排队。

### 6.5 冷热数据存储分层与清理周期
- 数据保留策略在管理面板 **Data Retention**（数据保留）页面按站点配置；清理循环按有界批次删除，大表不会锁库。
- `cold_ttl_ms` (`604800000`，即 7 天)：冷原始报文（`probe_cold`）超过该时长后被清理。
- `cold_promote_window_ms` (`864000000`，即 24 小时)：冷会话在该窗口内可被重新拉起（升热）而不是另起新会话。
- `cold_purge_interval_ms` (`300000`，即 5 分钟)：冷存储清理扫描周期。
- `ops_retention_days` (`14`，管理设置)：运维日志（`ops_client_events` 等运维表）保留天数。

### 6.6 分析触发条件与客户端重试阶梯
- `analyze_debounce_ms` (`40`)：相邻上报批次合并去抖窗口。
- `analyze_idle_upload_ms` (`20000`)：会话静止多久后触发最终结果裁决。
- `upload_concurrency` (`6`)：前端上传队列初始并发数。
- `upload_max_retries` (`5`)：网络抖动时的指数退避最大重试轮次。

---

## 7. 前端探针集成模式

### 7.1 域模型（pv / gv）
生产部署将探测流量拆分到两个第一方域名：
- **pv 域名**（`pv.yourdomain.com`）：会话生命周期——承载 `/gr.js`、`/dist/*` 与 `/v1`（open / ingest / result）。
- **gv 域名**（`gv.yourdomain.com`）：密封报文上传——`/v1/ingest/sealed`，带 TLS 客户端绑定。**上传始终由浏览器直连绑定的 gv 域名走 TLS**；不要将该路径再经其它源代理，否则密封绑定校验失败。

探针脚本支持的全部属性：
| 属性 | 必填 | 含义 |
|---|:---:|---|
| `data-site-id` | ✓ | 站点绑定（管理面板获取） |
| `data-endpoint` | — | pv 基础 URL（会话 open / 结果查询）。缺省回退到注入路径下的同源地址 |
| `data-inject-path` | — | `/gr.js` 自定义挂载路径（默认 `/gr.js`） |
| `data-gw-base` | — | gv 上传基础 URL 覆盖（默认使用会话授权下发的绑定 gv 域名） |

### 7.2 模式一：HTML 标签直接引入
```html
<script
  src="https://pv.yourdomain.com/gr.js"
  data-site-id="site_prod_90b21e"
  data-endpoint="https://pv.yourdomain.com"
  defer>
</script>
```

### 7.3 模式二：Nginx 第一方反向代理（推荐最佳实践）
将 **pv** 域名经 nginx 同源代理可彻底免除 CORS 跨域预检、天然绕过大部分广告拦截插件，并允许缓存静态资源。gv 上传路径**不经代理**——浏览器直连绑定的 gv 域名：

```nginx
# pv.yourdomain.com — 第一方代理到探测面（28765）
location = /gr.js {
    proxy_pass http://127.0.0.1:28765/gr.js;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    add_header Cache-Control "no-store" always;   # fe_epoch 新鲜度
}

location /dist/ {
    proxy_pass http://127.0.0.1:28765/dist/;
    proxy_set_header Host $host;
    add_header Cache-Control "public, max-age=31536000, immutable" always;  # 内容哈希
}

location /v1/ {
    proxy_pass http://127.0.0.1:28765/v1/;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    client_max_body_size 1m;
}
```

前置代理时，把代理网段加入 `/opt/greenpng/.env` 的 `GR_TRUSTED_PROXIES`，使真实 IP 与限流按访客而非代理记账（见 §6.3）。

### 7.4 模式三：Cloudflare Worker 注入
在自有域名的 Worker 上承接 `/gr.js` 并在 HTML 响应中注入脚本标签：

```js
// pv.yourdomain.com/gr.js — Worker 透传
export default {
  async fetch(req) {
    const upstream = await fetch("https://<origin>:28765/gr.js", {
      headers: { Host: new URL(req.url).hostname },
    });
    const resp = new Response(upstream.body, upstream);
    resp.headers.set("Cache-Control", "no-store");
    return resp;
  },
};
```
注入标签使用与模式一相同的 `data-site-id` / `data-endpoint` 属性。

### 7.5 模式四：应用模板内嵌
由服务端模板（Twig / Jinja / EJS / Thymeleaf）渲染脚本标签，而非改静态 HTML——属性同模式一，注入在 `</body>` 之前。

### 7.6 部署建议
- **先配 Cookie 白名单**：站点未配置业务 Cookie 白名单时，探针无法把会话与业务身份关联——上线前必须先配好。
- **缓存策略**：`/gr.js` 必须保持 `no-store`（携带当前 `fe_epoch`）；`/dist/*` 为内容哈希资源，可按 `immutable` 缓存一年；`/v1/*` 永不缓存。
- **广告拦截对抗**：第一方路径 + 中性文件名（模式二/三）可避开绝大多数过滤规则；避免使用第三方脚本域。
- **接入后冒烟自检**：`curl -s https://pv.yourdomain.com/v1/health`（或 `http://127.0.0.1:28765/v1/health`）应返回 200，随后真实浏览器打开一次页面，确认管理面板出现新会话。

---

## 8. 多语言 SDK 接入示范

SDK 属于纯后端客户端，用于在用户关键业务节点（注册、登录、交易、领券）查询风控判定：

### Go 语言接入
```go
package main

import (
    "log"

    "yourapp/vendor/grresults" // sdk/go/ 单文件客户端（package grresults），拷入项目即可
)

func main() {
    client := grresults.New("https://pv.yourdomain.com", "grsk_58f7a90b4e2d")
    verdict, err := client.WaitForResult("sess_91bf20a4ce", "sdk", 8000, 250)
    if err != nil {
        log.Fatalf("Query failed: %v", err)
    }

    if verdict["status"] == "bot" {
        // 触发人机验证码或直接拦截
    }
}
```

### TypeScript / Node.js 接入
```typescript
import { GrResultClient } from "@greenpng/results";

const client = new GrResultClient({
  baseUrl: "https://pv.yourdomain.com",
  apiKey: process.env.GREENPNG_SITE_KEY!,
});

const verdict = await client.waitForResult("sess_91bf20a4ce", { projection: "sdk" });
console.log(`Status: ${verdict.status}`);
```

---

## 9. 开源项目引用与致谢

greenpng 项目的实现建立在全球优秀开源项目与前沿网络安全研究成果之上：

### 核心基础设施与运行时
| 开源项目 | 开源协议 | 在 greenpng 中的应用与定位 |
|---|---|---|
| [Cloudflare Pingora](https://github.com/cloudflare/pingora) | Apache-2.0 | 高性能 Rust 网络代理框架，驱动 `gr-probe-plane` 的 TLS 握手与亚毫秒密封报文入库（采用其 `openssl` 特性构建）。 |
| [Tokio](https://github.com/tokio-rs/tokio) / [Axum](https://github.com/tokio-rs/axum) | MIT | 控制平面的异步运行时基座与高性能 REST API 框架。 |
| [OpenSSL](https://www.openssl.org/) | Apache-2.0 | Pingora 边缘与管理控制面的 TLS 后端。 |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | BSD-3 | 探针不可变资产清单签名、OTA 升级包验签与密文信封防重放。 |
| [PostgreSQL](https://www.postgresql.org/) / [Redis](https://redis.io/) | PG / RSALv2 | L3 关系型数据存根与多节点分布式集群状态同步。 |
| [Element Plus](https://element-plus.org/) / [Vue 3](https://vuejs.org/) | MIT | 管理控制台的现代桌面端 UI 组件库基座。 |

### 安全研究与前沿参考致谢
- **[CreepJS](https://github.com/abrahamjuliot/creepjs)**：全球领先的浏览器反指纹与原型链篡改检测研究。greenpng 的 B1 冲突检测与 B12 反伪装算法深受其特征隔离技术的启发。
- **[FingerprintJS](https://github.com/fingerprintjs/fingerprintjs)**：浏览器端软硬件特征提取领域的开拓者。
- **[BotD](https://github.com/fingerprintjs/botd)**：开源自动化框架（Puppeteer、Playwright 等）检测逻辑参考。

---

## 10. 安全与隐私合规承诺

- **数字签名护航**：所有官方发布的二进制及 OTA 模块均经 Ed25519 根密钥离线签名，杜绝供应链投毒。
- **发布资产不可变**：严格遵循语义化版本（SemVer），已发布 Tag 绝不强制覆写。
- **安全漏洞通报**：若发现任何安全隐患，请查阅 `SECURITY.md` 或直接与安全核心组取得联系。

---

<p align="center">
  <sub>&copy; 2026 greenpng Project. 基于 MIT 开源协议发布。</sub>
</p>
