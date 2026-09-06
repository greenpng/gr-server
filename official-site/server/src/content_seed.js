/**
 * Published marketing copy for greenpng (EN + zh-CN).
 * Capabilities: fingerprint-browser, disguise-browser, RPA, bot, batch-user, device-ID.
 */

export const BRAND = "greenpng";

export const PRICING_FAQ = {
  en: {
    items: [
      {
        q: "What does greenpng detect?",
        a: "Fingerprint browsers, disguised/spoofed browsers, RPA, bot users, batch/farmed accounts, and stable device IDs — without blocking your live traffic.",
      },
      {
        q: "What is included in the Free plan?",
        a: "Free is $0 per site. You get fingerprint-browser detection, disguise-browser detection, bot detection, batch-user detection, and device ID at standard precision (dv4 / dv5 / dv6). RPA detection is not included.",
      },
      {
        q: "What does Paid include, and how much is it?",
        a: "Paid is $99 per site per year. It unlocks highest-precision device ID (dv0) plus RPA detection, on top of every Free capability. Each site is billed separately.",
      },
      {
        q: "Does greenpng intercept or block visitors?",
        a: "No. greenpng is analysis-only. Scores and device IDs appear in your panel so your team can decide — there is no WAF-style edge block.",
      },
      {
        q: "Can I manage multiple sites?",
        a: "Yes. Add each domain as its own site. Free sites stay $0; upgrade only the sites that need dv0 + RPA at $99/year.",
      },
      {
        q: "When can I register?",
        a: "Public registration and sign-in are temporarily closed while we finish the commercial launch. The login page remains visible; account creation will open here.",
      },
    ],
  },
  "zh-CN": {
    items: [
      {
        q: "greenpng 检测什么？",
        a: "指纹浏览器、伪装/伪造浏览器、RPA、机器人用户、批量/群控账号，以及稳定的设备 ID。分析在面板完成，不拦截线上流量。",
      },
      {
        q: "免费版包含什么、多少钱？",
        a: "免费版 $0 / 站点。包含指纹浏览器检测、伪装浏览器检测、机器人检测、批量用户检测，以及标准精度设备 ID（dv4 / dv5 / dv6）。不含 RPA 检测。",
      },
      {
        q: "付费版包含什么、如何计费？",
        a: "付费版 $99 / 站点 / 年。在免费能力之上解锁最高精度设备 ID（dv0）与 RPA 检测。每个站点单独计费。",
      },
      {
        q: "会拦截或阻断访客吗？",
        a: "不会。greenpng 只做分析：评分与设备 ID 在面板展示，由你决策，没有 WAF 式边缘拦截。",
      },
      {
        q: "可以管理多个站点吗？",
        a: "可以。每个域名作为独立站点。免费站保持 $0；只需为需要 dv0 + RPA 的站点按 $99/年升级。",
      },
      {
        q: "什么时候可以注册？",
        a: "公开注册与登录暂时关闭，商业上线准备中。登录页仍可访问，开通后可在本站完成开户。",
      },
    ],
  },
};

export const HOME_MARKETING_BODY = {
  en: {
    blocks: [
      {
        type: "hero",
        badge: "Detection · analysis-only · no traffic blocking",
        headline: "Detect fingerprint browsers, bots, RPA, and device farms.",
        subhead:
          "greenpng is visitor and device detection for modern sites: fingerprint-browser detection, disguise-browser detection, RPA detection, bot-user detection, batch-user detection, and device ID. Analyze in-process — never intercept the edge.",
        cta_primary: { label: "See capabilities", href: "/features" },
        cta_secondary: { label: "View pricing", href: "/pricing" },
      },
      {
        type: "logo_strip",
        label: "Built for teams who need signal, not a WAF",
        items: ["E-commerce", "Fintech", "SaaS", "Media", "Gaming", "Ads"],
      },
      {
        type: "stats",
        items: [
          { value: "6", label: "Core detection services" },
          { value: "dv0–dv6", label: "Device ID precision lanes" },
          { value: "$99", label: "Paid site / year" },
          { value: "$0", label: "Free site forever" },
        ],
      },
      {
        type: "section_header",
        eyebrow: "What greenpng does",
        title: "Six detection services, one device ID",
        subtitle: "Identify risky automation and returning devices — then decide in your panel.",
      },
      {
        type: "features",
        items: [
          {
            icon: "shield",
            title: "Fingerprint browser detection",
            body: "Spot anti-detect / fingerprint browsers that rotate canvas, WebGL, audio, and fonts to look like unique devices.",
          },
          {
            icon: "layers",
            title: "Disguise browser detection",
            body: "Catch spoofed UA, forged Client Hints, mismatched OS/BR, and other disguise stacks that hide the real runtime.",
          },
          {
            icon: "zap",
            title: "RPA detection",
            body: "Paid: detect robotic process automation — scripted clicks, headless drivers, and replayed input that is not a human session.",
          },
          {
            icon: "chart",
            title: "Bot-user detection",
            body: "Score non-human traffic: crawlers, headless browsers, and scripted accounts that fail real-user consistency.",
          },
          {
            icon: "globe",
            title: "Batch-user detection",
            body: "Link farmed, multi-open, and bulk-registered identities that share hardware, timing, or environment families.",
          },
          {
            icon: "lock",
            title: "Device ID",
            body: "Stable device identifiers across visits. Free uses dv4–dv6; Paid unlocks dv0 for the highest-fidelity device ID.",
          },
        ],
      },
      {
        type: "section_header",
        eyebrow: "How it works",
        title: "Observe. Score. Decide.",
        subtitle: "greenpng never blocks the request path. Your product stays up while you investigate.",
      },
      {
        type: "steps",
        items: [
          {
            step: "01",
            title: "Connect a site",
            body: "Point greenpng at your domain. Free sites start with fingerprint, disguise, bot, batch, and device ID at standard precision.",
          },
          {
            step: "02",
            title: "Collect signals",
            body: "In-process analysis builds a device ID and detection scores — no edge WAF, no dropped checkout.",
          },
          {
            step: "03",
            title: "Review in the panel",
            body: "Inspect fingerprint-browser, disguise, RPA, bot, and batch-user hits. Upgrade a site to $99/year when you need dv0 + RPA.",
          },
        ],
      },
      {
        type: "section_header",
        eyebrow: "Pricing",
        title: "Free to start. $99/site/year for dv0 + RPA.",
        subtitle: "Transparent per-site billing. Public signup is temporarily closed.",
      },
      {
        type: "pricing",
        plans: [
          {
            id: "free",
            name: "Free",
            price: "$0",
            features: [
              "Fingerprint browser detection",
              "Disguise browser detection",
              "Bot-user detection",
              "Batch-user detection",
              "Device ID dv4 / dv5 / dv6",
              "No RPA detection",
            ],
            cta: { label: "Read the plans", href: "/pricing" },
          },
          {
            id: "paid",
            name: "Paid",
            price: "$99/site/yr",
            highlight: true,
            badge: "Popular",
            features: [
              "Everything in Free",
              "Device ID dv0 (highest precision)",
              "RPA detection enabled",
              "Per-site billing, cancel anytime at term end",
            ],
            cta: { label: "Pricing details", href: "/pricing" },
          },
        ],
      },
      {
        type: "cta_banner",
        headline: "See how greenpng scores your traffic",
        subhead: "Registration is temporarily closed. Browse capabilities, docs, and pricing — accounts will open on this site.",
        cta_primary: { label: "Explore features", href: "/features" },
        cta_secondary: { label: "Compare plans", href: "/pricing" },
      },
    ],
  },
  "zh-CN": {
    blocks: [
      {
        type: "hero",
        badge: "检测 · 只做分析 · 不拦截流量",
        headline: "检测指纹浏览器、机器人、RPA 与设备农场",
        subhead:
          "greenpng 面向现代站点的访客与设备检测：指纹浏览器检测、伪装浏览器检测、RPA 检测、机器人用户检测、批量用户检测、设备 ID 标识。进程内分析，边缘不拦截。",
        cta_primary: { label: "查看能力", href: "/zh-cn/features" },
        cta_secondary: { label: "查看定价", href: "/zh-cn/pricing" },
      },
      {
        type: "logo_strip",
        label: "为需要信号、而不是 WAF 的团队打造",
        items: ["电商", "金融科技", "SaaS", "媒体", "游戏", "广告"],
      },
      {
        type: "stats",
        items: [
          { value: "6", label: "核心检测服务" },
          { value: "dv0–dv6", label: "设备 ID 精度档" },
          { value: "$99", label: "付费站 / 年" },
          { value: "$0", label: "免费站长期免费" },
        ],
      },
      {
        type: "section_header",
        eyebrow: "greenpng 做什么",
        title: "六项检测，一个设备 ID",
        subtitle: "识别风险自动化与回访设备 — 在面板中由你决策。",
      },
      {
        type: "features",
        items: [
          {
            icon: "shield",
            title: "指纹浏览器检测",
            body: "识别抗指纹 / 指纹浏览器：通过 Canvas、WebGL、音频、字体等轮换伪装成「新设备」。",
          },
          {
            icon: "layers",
            title: "伪装浏览器检测",
            body: "识别伪造 UA、Client Hints、操作系统/浏览器不一致等伪装栈，揭开真实运行环境。",
          },
          {
            icon: "zap",
            title: "RPA 检测",
            body: "付费能力：检测流程自动化 — 脚本点击、无头驱动、回放输入等非真人会话。",
          },
          {
            icon: "chart",
            title: "机器人用户检测",
            body: "给非人类流量打分：爬虫、无头浏览器、脚本注册账号等无法保持真人一致性的访问。",
          },
          {
            icon: "globe",
            title: "批量用户检测",
            body: "关联群控、多开、批量注册：共享硬件、时序或环境家族的账号集群。",
          },
          {
            icon: "lock",
            title: "设备 ID 标识",
            body: "跨访问的稳定设备标识。免费为 dv4–dv6；付费解锁 dv0 最高精度设备 ID。",
          },
        ],
      },
      {
        type: "section_header",
        eyebrow: "如何工作",
        title: "观测、评分、决策",
        subtitle: "greenpng 不阻断请求路径。业务照常，调查在面板完成。",
      },
      {
        type: "steps",
        items: [
          {
            step: "01",
            title: "接入站点",
            body: "将 greenpng 指向你的域名。免费站即可使用指纹、伪装、机器人、批量用户检测与标准精度设备 ID。",
          },
          {
            step: "02",
            title: "采集信号",
            body: "进程内分析生成设备 ID 与检测评分 — 没有边缘 WAF，也不会丢掉下单请求。",
          },
          {
            step: "03",
            title: "面板复核",
            body: "查看指纹浏览器、伪装、RPA、机器人与批量用户命中。需要 dv0 + RPA 时，按 $99/年升级该站点。",
          },
        ],
      },
      {
        type: "section_header",
        eyebrow: "价格",
        title: "免费起步，dv0 + RPA 为 $99/站/年",
        subtitle: "按站点透明计费。公开注册暂时关闭。",
      },
      {
        type: "pricing",
        plans: [
          {
            id: "free",
            name: "免费",
            price: "$0",
            features: [
              "指纹浏览器检测",
              "伪装浏览器检测",
              "机器人用户检测",
              "批量用户检测",
              "设备 ID dv4 / dv5 / dv6",
              "不含 RPA 检测",
            ],
            cta: { label: "查看方案", href: "/zh-cn/pricing" },
          },
          {
            id: "paid",
            name: "付费",
            price: "$99/站/年",
            highlight: true,
            badge: "推荐",
            features: [
              "包含免费版全部能力",
              "设备 ID dv0（最高精度）",
              "开启 RPA 检测",
              "按站点计费，到期可不续",
            ],
            cta: { label: "价格说明", href: "/zh-cn/pricing" },
          },
        ],
      },
      {
        type: "cta_banner",
        headline: "看看 greenpng 如何给流量评分",
        subhead: "注册暂时关闭。可先浏览能力、教程与定价 — 开户将在本站开放。",
        cta_primary: { label: "浏览功能", href: "/zh-cn/features" },
        cta_secondary: { label: "对比方案", href: "/zh-cn/pricing" },
      },
    ],
  },
};

export const PRICING_BODY = {
  en: {
    blocks: [
      {
        type: "section_header",
        eyebrow: "Plans",
        title: "Simple per-site pricing",
        subtitle: "Free forever at $0. Paid is $99 USD per site per year — billed independently.",
      },
      {
        type: "pricing",
        plans: [
          {
            id: "free",
            name: "Free",
            price: "$0",
            features: [
              "Fingerprint browser detection",
              "Disguise browser detection",
              "Bot-user detection",
              "Batch-user detection",
              "Device ID: dv4 / dv5 / dv6",
              "Analysis-only (no edge block)",
              "RPA detection: not included",
            ],
            cta: { label: "Capabilities", href: "/features" },
          },
          {
            id: "paid",
            name: "Paid",
            price: "$99 / site / year",
            highlight: true,
            badge: "Recommended",
            features: [
              "All Free detection services",
              "Device ID: dv0 + dv4 / dv5 / dv6",
              "RPA detection enabled",
              "Highest-fidelity device identity",
              "Per-site upgrade — other sites can stay Free",
              "USD $99 billed yearly per upgraded site",
            ],
            cta: { label: "Read FAQ", href: "/pricing" },
          },
        ],
      },
      {
        type: "markdown",
        markdown:
          "## What’s billed\n\n| | Free | Paid |\n|---|---|---|\n| Price | **$0** / site | **$99 USD** / site / year |\n| Fingerprint browser detection | Yes | Yes |\n| Disguise browser detection | Yes | Yes |\n| Bot-user detection | Yes | Yes |\n| Batch-user detection | Yes | Yes |\n| Device ID | dv4 / dv5 / dv6 | **dv0** + dv4 / dv5 / dv6 |\n| RPA detection | No | **Yes** |\n\nPublic registration is temporarily closed. When accounts open, you will create a site on this website and upgrade only the domains that need Paid.",
      },
      { type: "faq", items: PRICING_FAQ.en.items },
      {
        type: "cta_banner",
        headline: "Questions before launch?",
        subhead: "Read the docs and changelog. Signup will be enabled on this same login page.",
        cta_primary: { label: "Documentation", href: "/docs" },
        cta_secondary: { label: "Changelog", href: "/changelog" },
      },
    ],
  },
  "zh-CN": {
    blocks: [
      {
        type: "section_header",
        eyebrow: "方案",
        title: "按站点计费，价格清楚",
        subtitle: "免费长期 $0。付费为每站点每年 99 美元，彼此独立。",
      },
      {
        type: "pricing",
        plans: [
          {
            id: "free",
            name: "免费",
            price: "$0",
            features: [
              "指纹浏览器检测",
              "伪装浏览器检测",
              "机器人用户检测",
              "批量用户检测",
              "设备 ID：dv4 / dv5 / dv6",
              "只做分析（不拦截边缘）",
              "不含 RPA 检测",
            ],
            cta: { label: "功能说明", href: "/zh-cn/features" },
          },
          {
            id: "paid",
            name: "付费",
            price: "$99 / 站 / 年",
            highlight: true,
            badge: "推荐",
            features: [
              "包含免费版全部检测",
              "设备 ID：dv0 + dv4 / dv5 / dv6",
              "开启 RPA 检测",
              "最高精度设备标识",
              "按站点升级 — 其余站点可继续免费",
              "每个升级站点每年 99 美元",
            ],
            cta: { label: "阅读常见问题", href: "/zh-cn/pricing" },
          },
        ],
      },
      {
        type: "markdown",
        markdown:
          "## 计费对照\n\n| | 免费 | 付费 |\n|---|---|---|\n| 价格 | **$0** / 站点 | **$99 美元** / 站点 / 年 |\n| 指纹浏览器检测 | 有 | 有 |\n| 伪装浏览器检测 | 有 | 有 |\n| 机器人用户检测 | 有 | 有 |\n| 批量用户检测 | 有 | 有 |\n| 设备 ID | dv4 / dv5 / dv6 | **dv0** + dv4 / dv5 / dv6 |\n| RPA 检测 | 无 | **有** |\n\n公开注册暂时关闭。开通账户后，在本站创建站点，只为需要付费能力的域名升级。",
      },
      { type: "faq", items: PRICING_FAQ["zh-CN"].items },
      {
        type: "cta_banner",
        headline: "上线前想先了解？",
        subhead: "可阅读教程与更新日志。注册将在同一登录页开放。",
        cta_primary: { label: "使用教程", href: "/zh-cn/docs" },
        cta_secondary: { label: "更新日志", href: "/zh-cn/changelog" },
      },
    ],
  },
};

export const FEATURES_BODY = {
  en: {
    blocks: [
      {
        type: "section_header",
        eyebrow: "Platform",
        title: "greenpng detection services",
        subtitle: "Fingerprint browsers, disguise browsers, RPA, bots, batch users, and device ID — analysis only.",
      },
      {
        type: "features",
        items: [
          {
            icon: "shield",
            title: "Fingerprint browser detection",
            body: "Anti-detect browsers randomize fingerprints to evade tracking. greenpng flags inconsistent hardware/software fingerprints across sessions.",
          },
          {
            icon: "layers",
            title: "Disguise browser detection",
            body: "Spoofed user-agents, forged client hints, and OS/BR mismatches are scored as disguise — even when the page looks “normal”.",
          },
          {
            icon: "zap",
            title: "RPA detection (Paid)",
            body: "Robotic process automation leaves timing, input, and driver residue. Paid sites enable the RPA probe path gated by entitlement.",
          },
          {
            icon: "chart",
            title: "Bot-user detection",
            body: "Separate humans from crawlers, headless runtimes, and scripted accounts using environment plus behavior consistency.",
          },
          {
            icon: "globe",
            title: "Batch-user detection",
            body: "Cluster multi-open, device-farm, and bulk-registration patterns that share a device family or environment.",
          },
          {
            icon: "lock",
            title: "Device ID",
            body: "A stable identifier for returning hardware. Free: dv4–dv6. Paid: dv0 for highest-precision device identity.",
          },
        ],
      },
      {
        type: "markdown",
        markdown:
          "## Analysis-only by design\n\n- **No request interception** at the CDN or reverse proxy\n- Scores and device IDs are for **your panel**, not an automatic ban\n- Algorithm bundles are delivered with **ECDH + Ed25519** from the official site\n- Entitlements (Free vs Paid / RPA) are enforced per site\n\nStart with [pricing](/pricing) or the [getting started](/docs/getting-started) guide.",
      },
      {
        type: "cta_banner",
        headline: "Map detections to a plan",
        subhead: "Free covers five services at standard device ID. Paid adds dv0 and RPA for $99/site/year.",
        cta_primary: { label: "Pricing", href: "/pricing" },
        cta_secondary: { label: "Docs", href: "/docs" },
      },
    ],
  },
  "zh-CN": {
    blocks: [
      {
        type: "section_header",
        eyebrow: "平台",
        title: "greenpng 检测服务",
        subtitle: "指纹浏览器、伪装浏览器、RPA、机器人、批量用户与设备 ID — 只做分析。",
      },
      {
        type: "features",
        items: [
          {
            icon: "shield",
            title: "指纹浏览器检测",
            body: "抗指纹浏览器通过轮换指纹规避追踪。greenpng 标记跨会话不一致的软硬件指纹。",
          },
          {
            icon: "layers",
            title: "伪装浏览器检测",
            body: "伪造 UA、Client Hints、操作系统/浏览器不一致会记为伪装 — 即使页面看起来「正常」。",
          },
          {
            icon: "zap",
            title: "RPA 检测（付费）",
            body: "流程自动化会留下时序、输入与驱动残留。付费站开启由 entitlement 控制的 RPA 探针。",
          },
          {
            icon: "chart",
            title: "机器人用户检测",
            body: "结合环境与行为一致性，把真人与爬虫、无头运行时、脚本账号分开。",
          },
          {
            icon: "globe",
            title: "批量用户检测",
            body: "聚类多开、设备农场、批量注册：共享设备家族或环境的账号。",
          },
          {
            icon: "lock",
            title: "设备 ID 标识",
            body: "回访硬件的稳定标识。免费：dv4–dv6。付费：dv0 最高精度设备身份。",
          },
        ],
      },
      {
        type: "markdown",
        markdown:
          "## 只做分析\n\n- CDN / 反向代理上**不拦截请求**\n- 评分与设备 ID 给**你的面板**，不是自动封禁\n- 算法包由官网 **ECDH + Ed25519** 下发\n- 免费 / 付费 / RPA entitlement **按站点**生效\n\n可先看[定价](/zh-cn/pricing)或[快速上手](/zh-cn/docs/getting-started)。",
      },
      {
        type: "cta_banner",
        headline: "把检测能力对应到套餐",
        subhead: "免费覆盖五项服务与标准设备 ID。付费增加 dv0 与 RPA，每站每年 $99。",
        cta_primary: { label: "定价", href: "/zh-cn/pricing" },
        cta_secondary: { label: "教程", href: "/zh-cn/docs" },
      },
    ],
  },
};

function md(enTitle, enSub, enMeta, enBody, zhTitle, zhSub, zhMeta, zhBody, slug) {
  return {
    en: {
      slug,
      title: enTitle,
      subtitle: enSub,
      meta_title: enMeta,
      meta_description: enSub,
      body: { blocks: [{ type: "markdown", markdown: enBody }] },
    },
    "zh-CN": {
      slug,
      title: zhTitle,
      subtitle: zhSub,
      meta_title: zhMeta,
      meta_description: zhSub,
      body: { blocks: [{ type: "markdown", markdown: zhBody }] },
    },
  };
}

export const MARKETING_PAGE_META = {
  home: {
    sort_order: 0,
    kind: "marketing",
    en: {
      slug: "",
      title: "Detect fingerprint browsers, bots, RPA, and device farms",
      subtitle: "greenpng: fingerprint-browser, disguise-browser, RPA, bot, batch-user detection, and device ID — analysis only.",
      meta_title: "greenpng — fingerprint, bot, RPA & device ID detection",
      meta_description:
        "greenpng detects fingerprint browsers, disguised browsers, RPA, bots, batch users, and device IDs. Free $0; Paid $99/site/year for dv0 + RPA.",
      body: HOME_MARKETING_BODY.en,
    },
    "zh-CN": {
      slug: "",
      title: "检测指纹浏览器、机器人、RPA 与设备农场",
      subtitle: "greenpng：指纹浏览器、伪装浏览器、RPA、机器人、批量用户检测与设备 ID — 只做分析。",
      meta_title: "greenpng — 指纹浏览器、机器人、RPA 与设备 ID 检测",
      meta_description:
        "greenpng 提供指纹浏览器检测、伪装浏览器检测、RPA 检测、机器人用户检测、批量用户检测与设备 ID。免费 $0；付费 $99/站/年（dv0 + RPA）。",
      body: HOME_MARKETING_BODY["zh-CN"],
    },
  },
  features: {
    sort_order: 5,
    kind: "marketing",
    en: {
      slug: "features",
      title: "Features",
      subtitle: "Six detection services and device ID from greenpng",
      meta_title: "greenpng features — detection services",
      meta_description: "Fingerprint browser, disguise browser, RPA, bot, batch-user detection, and device ID.",
      body: FEATURES_BODY.en,
    },
    "zh-CN": {
      slug: "features",
      title: "功能",
      subtitle: "greenpng 的六项检测服务与设备 ID",
      meta_title: "greenpng 功能 — 检测服务",
      meta_description: "指纹浏览器、伪装浏览器、RPA、机器人、批量用户检测与设备 ID。",
      body: FEATURES_BODY["zh-CN"],
    },
  },
  pricing: {
    sort_order: 10,
    kind: "marketing",
    en: {
      slug: "pricing",
      title: "Pricing",
      subtitle: "Free $0. Paid $99 per site per year for dv0 device ID + RPA detection.",
      meta_title: "greenpng pricing — Free $0, Paid $99/site/year",
      meta_description: "Free includes five detections and standard device ID. Paid is $99/site/year for dv0 + RPA.",
      body: PRICING_BODY.en,
    },
    "zh-CN": {
      slug: "pricing",
      title: "定价",
      subtitle: "免费 $0。付费每站点每年 $99，含 dv0 设备 ID 与 RPA 检测。",
      meta_title: "greenpng 定价 — 免费 $0，付费 $99/站/年",
      meta_description: "免费含五项检测与标准设备 ID。付费 $99/站/年，含 dv0 + RPA。",
      body: PRICING_BODY["zh-CN"],
    },
  },
};

export const EXTRA_CONTENT_PAGES = [
  {
    page_key: "doc_getting_started",
    kind: "doc",
    sort_order: 20,
    locales: md(
      "Getting started",
      "How greenpng detection fits on your site.",
      "Getting started · greenpng",
      `## What you get

greenpng provides:

1. **Fingerprint browser detection**
2. **Disguise browser detection**
3. **RPA detection** (Paid)
4. **Bot-user detection**
5. **Batch-user detection**
6. **Device ID**

Traffic is **not blocked**. Results appear in the analysis panel.

## Plans

- **Free — $0 / site:** items 1, 2, 4, 5, and device ID at dv4–dv6. No RPA.
- **Paid — $99 / site / year:** everything in Free, plus **dv0 device ID** and **RPA detection**.

## Status of accounts

Public **register and sign-in are temporarily disabled**. You can still open the login page; you cannot complete those actions yet. When we enable accounts, you will:

1. Create an account on this website
2. Add a site (domain)
3. Review detections in the panel
4. Upgrade individual sites to Paid if you need dv0 + RPA

See [pricing](/pricing) and [fingerprint browser detection](/docs/fingerprint-browsers).`,
      "快速上手",
      "greenpng 检测如何接到你的站点。",
      "快速上手 · greenpng",
      `## 你将获得

greenpng 提供：

1. **指纹浏览器检测**
2. **伪装浏览器检测**
3. **RPA 检测**（付费）
4. **机器人用户检测**
5. **批量用户检测**
6. **设备 ID 标识**

**不拦截流量**。结果在分析面板中查看。

## 套餐

- **免费 — $0 / 站点：** 第 1、2、4、5 项，以及 dv4–dv6 设备 ID。不含 RPA。
- **付费 — $99 / 站点 / 年：** 免费全部能力，外加 **dv0 设备 ID** 与 **RPA 检测**。

## 账户状态

公开**注册与登录暂时关闭**。登录页仍可打开，但无法完成操作。开通后你可以：

1. 在本站创建账户
2. 添加站点（域名）
3. 在面板查看检测结果
4. 需要 dv0 + RPA 时，按站点升级付费

参见[定价](/zh-cn/pricing)与[指纹浏览器检测](/zh-cn/docs/fingerprint-browsers)。`,
      "getting-started"
    ),
  },
  {
    page_key: "doc_fingerprint_browsers",
    kind: "doc",
    sort_order: 21,
    locales: md(
      "Fingerprint & disguise browsers",
      "How greenpng treats anti-detect and spoofed runtimes.",
      "Fingerprint & disguise browsers · greenpng",
      `## Fingerprint browser detection

Fingerprint (anti-detect) browsers try to look like a unique, clean device on every profile: canvas, WebGL, audio, fonts, and more are randomized.

greenpng scores those inconsistencies against a real device ID so farms cannot cheaply mint “new users”.

## Disguise browser detection

Disguise stacks keep the same engine but **lie** about it: fake UA strings, forged Client Hints, OS/BR mismatches.

These two services are available on **Free and Paid**. Pair them with [bot and batch-user detection](/docs/bots-and-batch) and [device ID](/docs/device-id).`,
      "指纹浏览器与伪装浏览器",
      "greenpng 如何对待抗指纹与伪装运行时。",
      "指纹与伪装浏览器 · greenpng",
      `## 指纹浏览器检测

指纹（抗指纹）浏览器试图让每个配置文件都像一台干净的新设备：Canvas、WebGL、音频、字体等被轮换。

greenpng 把这些不一致对照真实设备 ID 评分，使设备农场难以廉价制造「新用户」。

## 伪装浏览器检测

伪装栈沿用同一引擎但**撒谎**：假 UA、伪造 Client Hints、操作系统/浏览器不一致。

这两项在**免费与付费**均可用。可配合[机器人与批量用户检测](/zh-cn/docs/bots-and-batch)以及[设备 ID](/zh-cn/docs/device-id)。`,
      "fingerprint-browsers"
    ),
  },
  {
    page_key: "doc_bots_batch",
    kind: "doc",
    sort_order: 22,
    locales: md(
      "Bots, batch users, and RPA",
      "Non-human and farmed traffic versus scripted automation.",
      "Bots, batch users, RPA · greenpng",
      `## Bot-user detection

Bots include crawlers, headless browsers, and scripted accounts. greenpng checks whether the session is consistent with a human runtime.

## Batch-user detection

Batch users share hardware, clocks, or environments — multi-open, device farms, bulk registration. Detection links those families even when cookies are cleared.

## RPA detection (Paid)

RPA (robotic process automation) drives the page like a macro: drivers, replayed events, non-human timing. **RPA is Paid only** ($99/site/year) together with dv0 device ID.

Free still includes bot and batch-user detection without the RPA probe.`,
      "机器人、批量用户与 RPA",
      "非人类流量、群控账号与脚本自动化。",
      "机器人、批量用户与 RPA · greenpng",
      `## 机器人用户检测

机器人包括爬虫、无头浏览器、脚本账号。greenpng 检查会话是否符合真人运行时。

## 批量用户检测

批量用户共享硬件、时钟或环境 — 多开、设备农场、批量注册。即使清 Cookie，也能把这些家族关联起来。

## RPA 检测（付费）

RPA（流程自动化）像宏一样驱动页面：驱动、回放事件、非人及时序。**RPA 仅付费**（$99/站/年），并与 dv0 设备 ID 一起提供。

免费版仍含机器人与批量用户检测，但不含 RPA 探针。`,
      "bots-and-batch"
    ),
  },
  {
    page_key: "doc_device_id",
    kind: "doc",
    sort_order: 23,
    locales: md(
      "Device ID",
      "Precision lanes dv0–dv6 and what Paid unlocks.",
      "Device ID · greenpng",
      `## Why device ID

A **device ID** is a stable handle for the hardware/runtime, used together with fingerprint, disguise, bot, and batch detections.

## Precision

| Plan | Lanes | Typical use |
|---|---|---|
| Free ($0) | dv4, dv5, dv6 | Standard returning-device identity |
| Paid ($99/site/year) | **dv0** + dv4, dv5, dv6 | Highest-fidelity device ID + RPA |

Higher precision is not “more blocking” — greenpng still does not intercept requests. It improves how confidently you can say “same device”.`,
      "设备 ID 标识",
      "精度档 dv0–dv6，以及付费解锁内容。",
      "设备 ID · greenpng",
      `## 为什么需要设备 ID

**设备 ID** 是硬件/运行时的稳定句柄，与指纹、伪装、机器人、批量检测一起使用。

## 精度

| 方案 | 档位 | 典型用途 |
|---|---|---|
| 免费（$0） | dv4、dv5、dv6 | 标准回访设备标识 |
| 付费（$99/站/年） | **dv0** + dv4、dv5、dv6 | 最高精度设备 ID + RPA |

更高精度不是「更多拦截」— greenpng 仍然不拦截请求。它提高的是你判断「同一台设备」的把握。`,
      "device-id"
    ),
  },
  {
    page_key: "blog_introducing_greenpng",
    kind: "blog",
    sort_order: 30,
    locales: md(
      "Introducing greenpng",
      "Detection for fingerprint browsers, bots, RPA, and device farms — without blocking traffic.",
      "Introducing greenpng",
      `**greenpng** is our product brand for visitor and device detection.

Teams that sell, advertise, or run accounts online need to know:

- Is this a **fingerprint browser**?
- Is the browser **disguised**?
- Is this **RPA** or a **bot**?
- Are these **batch users** from the same farm?
- What is the **device ID**?

greenpng answers those questions **in analysis**, not by dropping packets at the edge.

**Pricing:** Free sites are **$0**. Paid is **$99 per site per year** for dv0 device ID and RPA detection.

Public registration is temporarily closed; the [login page](/login) stays visible. Read [getting started](/docs/getting-started).`,
      "greenpng 发布",
      "指纹浏览器、机器人、RPA 与设备农场检测 — 不拦截流量。",
      "greenpng 发布",
      `**greenpng** 是我们的访客与设备检测产品品牌。

做交易、投放或账号运营的团队需要知道：

- 这是不是**指纹浏览器**？
- 浏览器是否被**伪装**？
- 这是 **RPA** 还是**机器人**？
- 这些是不是同一农场的**批量用户**？
- **设备 ID** 是什么？

greenpng 在**分析**中回答这些问题，而不是在边缘丢包。

**价格：** 免费站 **$0**。付费为每站点每年 **$99**，含 dv0 设备 ID 与 RPA 检测。

公开注册暂时关闭；[登录页](/zh-cn/login)仍可访问。阅读[快速上手](/zh-cn/docs/getting-started)。`,
      "introducing-greenpng"
    ),
  },
  {
    page_key: "blog_fingerprint_vs_disguise",
    kind: "blog",
    sort_order: 31,
    locales: md(
      "Fingerprint browsers vs disguise browsers",
      "Two different cheats — greenpng detects both.",
      "Fingerprint vs disguise browsers · greenpng",
      `A **fingerprint browser** tries to become a *new* device: it randomizes the fingerprint so trackers cannot stitch visits.

A **disguise browser** tries to become a *different* device class: it keeps one engine but spoofs UA, hints, and OS/BR.

Both show up in account abuse, ad fraud, and multi-accounting. greenpng runs **fingerprint browser detection** and **disguise browser detection** on Free and Paid, then binds them to a **device ID**.

If the operator also scripts the UI, Paid **RPA detection** is the additional signal. Details: [docs](/docs/fingerprint-browsers).`,
      "指纹浏览器 vs 伪装浏览器",
      "两种不同的作弊方式 — greenpng 都能检。",
      "指纹浏览器与伪装浏览器 · greenpng",
      `**指纹浏览器**试图变成一台*新*设备：轮换指纹，让追踪无法把访问串起来。

**伪装浏览器**试图变成另一种*设备类型*：引擎不变，但伪造 UA、Hints 和操作系统/浏览器。

两者常见于账号滥用、广告欺诈和多开。greenpng 在免费与付费上都提供**指纹浏览器检测**和**伪装浏览器检测**，并绑定到**设备 ID**。

如果操作者还在脚本化界面，付费 **RPA 检测**是额外信号。详见[教程](/zh-cn/docs/fingerprint-browsers)。`,
      "fingerprint-vs-disguise"
    ),
  },
  {
    page_key: "blog_batch_users",
    kind: "blog",
    sort_order: 32,
    locales: md(
      "Why batch-user detection matters",
      "Cleared cookies are not a new customer.",
      "Batch-user detection · greenpng",
      `Bulk registration and device farms often look like many users in your product database. They share:

- the same device family (**device ID**)
- the same anti-detect profile (**fingerprint browser detection**)
- scripted onboarding (**RPA** / **bots**)

**Batch-user detection** is the layer that says “these accounts move together”. It is included on Free. Combine it with Paid RPA when the farm is fully automated.

[Pricing](/pricing) · [Bots & batch docs](/docs/bots-and-batch)`,
      "为什么需要批量用户检测",
      "清掉 Cookie 并不等于新客户。",
      "批量用户检测 · greenpng",
      `批量注册和设备农场在业务库里常常像很多用户。它们共享：

- 同一设备家族（**设备 ID**）
- 同一抗指纹配置（**指纹浏览器检测**）
- 脚本化开户（**RPA** / **机器人**）

**批量用户检测**要回答的是「这些账号在一起动」。免费版包含此项。农场全自动时，再叠加付费 RPA。

[定价](/zh-cn/pricing) · [机器人与批量教程](/zh-cn/docs/bots-and-batch)`,
      "batch-user-detection"
    ),
  },
  {
    page_key: "changelog_2026_08_14",
    kind: "changelog",
    sort_order: 39,
    locales: md(
      "2026-08-14 — greenpng positioning",
      "Brand, six detection services, pricing copy, docs/blog, public auth paused.",
      "Changelog 2026-08-14 · greenpng",
      `### Added

- Product brand **greenpng** on the official site (EN + zh-CN)
- Explicit services: fingerprint browser, disguise browser, RPA, bot-user, batch-user, device ID
- Docs: getting started, fingerprint/disguise, bots/batch/RPA, device ID
- Blog posts introducing the brand and detection topics
- Pricing table: Free **$0**, Paid **$99 / site / year** (dv0 + RPA)

### Changed

- Public **register and login** remain visible but cannot complete (launch hold)

### Notes

- Analysis-only: no edge traffic blocking`,
      "2026-08-14 — greenpng 定位",
      "品牌、六项检测、定价文案、教程/博客，公开登录暂停。",
      "更新日志 2026-08-14 · greenpng",
      `### 新增

- 官网产品品牌 **greenpng**（中英）
- 明确服务：指纹浏览器、伪装浏览器、RPA、机器人用户、批量用户、设备 ID
- 教程：快速上手、指纹/伪装、机器人/批量/RPA、设备 ID
- 博客：品牌与检测主题
- 价格表：免费 **$0**，付费 **$99 / 站 / 年**（dv0 + RPA）

### 变更

- 公开**注册与登录**页面可见，但无法完成操作（上线前暂停）

### 说明

- 只做分析：不拦截边缘流量`,
      "2026-08-14"
    ),
  },
  {
    page_key: "changelog_2026_08",
    kind: "changelog",
    sort_order: 40,
    locales: md(
      "2026-08-13 — Official site foundation",
      "Locale routing, CMS, docs & blog sections.",
      "Changelog 2026-08-13",
      `### Added

- English at site root (no \`/en\` prefix)
- Browser language auto-detect with \`zh\` → \`/zh-cn\`
- Blog, changelog, and documentation sections
- PostgreSQL CMS for bilingual pages

### Changed

- Mobile navigation and responsive layout`,
      "2026-08-13 — 官网基础",
      "语言路由、CMS、教程与博客分区。",
      "更新日志 2026-08-13",
      `### 新增

- 英文默认在根路径（无 \`/en\` 前缀）
- 浏览器语言自动识别（中文 → \`/zh-cn\`）
- 博客、更新日志、使用教程分区
- PostgreSQL 双语 CMS

### 变更

- 移动端导航与响应式布局`,
      "2026-08-13"
    ),
  },
];
