//! Product public projection layer (`product_public_v1`).
//!
//! # Purpose
//! Map internal evaluate materials → merchant-readable envelope:
//! `identity + scores + bot + signals[] + recommended_action`.
//!
//! # Redlines
//! - Does **not** mint or rewrite commercial digests (`select_device_segments`).
//! - Soft never promotes. UA/IP never become device_id.
//! - Full analyze still computes everything; this layer only **projects + decides**.
//!
//! # Strategy
//! Built-in presets (goal intensity, not business pages):
//! - `observe_only` / `shadow` — never block; `would_have_action`
//! - `balanced` / `default` / `hybrid` — **default** G1–G6 balanced (aliases: `login_strict`)
//! - `strict_integrity` — stricter AD/spoof (aliases: `checkout`)
//! - `bot_control` — crawler/automation hard deny (aliases: `content_bot`)
//! - `identity_focus` — device conf emphasis, fewer hard bans
//! - `register_anti_abuse` — farm/homogenization emphasis
//!
//! Admin passes `strategy_id` + `response_profile` without changing mint.
//!
//! # Python lab parity
//! Decision redlines mirror `/home/ubuntu/probe-algo-lab` R5 (`test_r5_redlines`,
//! hybrid_r4 philosophy). Lab is Python for fast iteration; **production truth is
//! this Rust module** — keep unit tests below in sync when changing either side.

use serde_json::{json, Map, Value};

pub const PRODUCT_PUBLIC_ALGO: &str = "product_public_v1";
pub const API_VERSION: &str = "v6.1";

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseProfile {
    Basic,
    Standard,
    Advanced,
    Diagnostic,
}

impl ResponseProfile {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "basic" | "starter" => Self::Basic,
            "advanced" | "enterprise" => Self::Advanced,
            "diagnostic" | "lab" | "debug" => Self::Diagnostic,
            _ => Self::Standard,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Standard => "standard",
            Self::Advanced => "advanced",
            Self::Diagnostic => "diagnostic",
        }
    }
    /// Clamp requested profile to a plan/key ceiling (request can only tighten).
    pub fn clamp_to(self, cap: Self) -> Self {
        let rank = |p: Self| match p {
            Self::Basic => 0,
            Self::Standard => 1,
            Self::Advanced => 2,
            Self::Diagnostic => 3,
        };
        if rank(self) > rank(cap) {
            cap
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyId {
    /// Shadow / gray release — never hard block.
    ObserveOnly,
    /// Default G1–G6 balanced detection (hard bot deny, AD deny, soft RPA never alone).
    Balanced,
    /// Alias retained for API compat (= Balanced thresholds).
    LoginStrict,
    /// Farm / multi-account emphasis.
    RegisterAntiAbuse,
    /// Strict integrity / high sensitivity (AD/spoof easier deny).
    StrictIntegrity,
    /// Alias retained for API compat (= StrictIntegrity).
    Checkout,
    /// Hard crawler/automation control.
    BotControl,
    /// Alias retained for API compat (= BotControl).
    ContentBot,
    /// Device identity emphasis; softer automation bans except webdriver.
    IdentityFocus,
}

impl StrategyId {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().replace('-', "_").as_str() {
            "observe_only" | "observe" | "shadow" => Self::ObserveOnly,
            "balanced" | "default" | "hybrid" | "hybrid_r4" | "leading" => Self::Balanced,
            "login_strict" | "login" => Self::LoginStrict,
            "register_anti_abuse" | "register" | "signup" => Self::RegisterAntiAbuse,
            "strict_integrity" | "integrity" | "ad_strict" => Self::StrictIntegrity,
            "checkout" | "payment" | "pay" => Self::Checkout,
            "bot_control" | "bot_protect" | "automation" => Self::BotControl,
            "content_bot" | "content" | "crawl" => Self::ContentBot,
            "identity_focus" | "identity" | "fp_smart" | "device" => Self::IdentityFocus,
            _ => Self::Balanced, // goal-aligned default when probe loads
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe_only",
            Self::Balanced => "balanced",
            Self::LoginStrict => "login_strict",
            Self::RegisterAntiAbuse => "register_anti_abuse",
            Self::StrictIntegrity => "strict_integrity",
            Self::Checkout => "checkout",
            Self::BotControl => "bot_control",
            Self::ContentBot => "content_bot",
            Self::IdentityFocus => "identity_focus",
        }
    }
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::ObserveOnly => "只观察（影子）",
            Self::Balanced => "均衡检测（默认·G1–G6）",
            Self::LoginStrict => "登录严格（别名→均衡）",
            Self::RegisterAntiAbuse => "反滥用/农场侧重",
            Self::StrictIntegrity => "完整性严格（假浏览器/AD）",
            Self::Checkout => "支付/下单（别名→完整性严格）",
            Self::BotControl => "机器人/爬虫控制",
            Self::ContentBot => "内容防爬（别名→机器人控制）",
            Self::IdentityFocus => "设备身份侧重",
        }
    }
    pub fn label_en(self) -> &'static str {
        match self {
            Self::ObserveOnly => "Observe only (shadow)",
            Self::Balanced => "Balanced (default · G1–G6)",
            Self::LoginStrict => "Login strict (alias → balanced)",
            Self::RegisterAntiAbuse => "Register anti-abuse / farm",
            Self::StrictIntegrity => "Strict integrity (spoof / AD)",
            Self::Checkout => "Checkout (alias → strict integrity)",
            Self::BotControl => "Bot / crawler control",
            Self::ContentBot => "Content anti-crawl (alias → bot control)",
            Self::IdentityFocus => "Device identity focus",
        }
    }
    /// Goal family for decision tables (aliases collapse).
    fn family(self) -> StrategyFamily {
        match self {
            Self::ObserveOnly => StrategyFamily::Observe,
            Self::Balanced | Self::LoginStrict => StrategyFamily::Balanced,
            Self::RegisterAntiAbuse => StrategyFamily::Register,
            Self::StrictIntegrity | Self::Checkout => StrategyFamily::StrictIntegrity,
            Self::BotControl | Self::ContentBot => StrategyFamily::BotControl,
            Self::IdentityFocus => StrategyFamily::IdentityFocus,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrategyFamily {
    Observe,
    Balanced,
    Register,
    StrictIntegrity,
    BotControl,
    IdentityFocus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicAction {
    Allow,
    Challenge,
    Deny,
    Observe,
}

impl PublicAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Challenge => "challenge",
            Self::Deny => "deny",
            Self::Observe => "observe",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrategyThresholds {
    pub device_confidence_lt: f64,
    pub os_score_lt: f64,
    pub br_score_lt: f64,
    pub rpa_score_lt: f64,
    pub risk_ge_challenge: f64,
    pub risk_ge_deny: f64,
    pub multi_signal_challenge: u32,
}

#[derive(Debug, Clone)]
pub struct StrategyPreset {
    pub id: StrategyId,
    pub sensitivity_sensitive: bool,
    pub probe_depth: &'static str,
    pub response_profile_default: ResponseProfile,
    pub thresholds: StrategyThresholds,
    pub shadow_mode: bool,
    /// Weight for risk fusion: (os, br, rpa, conf)
    pub risk_weights: (f64, f64, f64, f64),
}

impl StrategyPreset {
    pub fn builtin(id: StrategyId) -> Self {
        let base = StrategyThresholds {
            device_confidence_lt: 0.55,
            os_score_lt: 0.50,
            br_score_lt: 0.55,
            rpa_score_lt: 0.45,
            risk_ge_challenge: 0.55,
            risk_ge_deny: 0.85,
            multi_signal_challenge: 2,
        };
        // Collapse aliases to shared threshold families (API id still preserved).
        match id.family() {
            StrategyFamily::Observe => Self {
                id,
                sensitivity_sensitive: false,
                probe_depth: "lite",
                response_profile_default: ResponseProfile::Basic,
                thresholds: StrategyThresholds {
                    risk_ge_challenge: 1.01,
                    risk_ge_deny: 1.01,
                    multi_signal_challenge: 99,
                    ..base
                },
                shadow_mode: true,
                risk_weights: (0.35, 0.35, 0.20, 0.10),
            },
            StrategyFamily::Balanced => Self {
                id,
                sensitivity_sensitive: true,
                probe_depth: "standard",
                response_profile_default: ResponseProfile::Standard,
                thresholds: base,
                shadow_mode: false,
                // hybrid_r4 weights: os/br integrity + bot control + conf
                risk_weights: (0.30, 0.30, 0.25, 0.15),
            },
            StrategyFamily::Register => Self {
                id,
                sensitivity_sensitive: true,
                probe_depth: "standard",
                response_profile_default: ResponseProfile::Standard,
                thresholds: StrategyThresholds {
                    device_confidence_lt: 0.50,
                    risk_ge_challenge: 0.50,
                    risk_ge_deny: 0.80,
                    multi_signal_challenge: 2,
                    ..base
                },
                shadow_mode: false,
                risk_weights: (0.25, 0.25, 0.30, 0.20),
            },
            StrategyFamily::StrictIntegrity => Self {
                id,
                sensitivity_sensitive: true,
                probe_depth: "deep",
                response_profile_default: ResponseProfile::Advanced,
                thresholds: StrategyThresholds {
                    device_confidence_lt: 0.60,
                    os_score_lt: 0.55,
                    br_score_lt: 0.55,
                    rpa_score_lt: 0.50,
                    risk_ge_challenge: 0.45,
                    risk_ge_deny: 0.75,
                    multi_signal_challenge: 1,
                    ..base
                },
                shadow_mode: false,
                risk_weights: (0.35, 0.35, 0.15, 0.15),
            },
            StrategyFamily::BotControl => Self {
                id,
                sensitivity_sensitive: false,
                probe_depth: "lite",
                response_profile_default: ResponseProfile::Standard,
                thresholds: StrategyThresholds {
                    device_confidence_lt: 0.40,
                    os_score_lt: 0.35,
                    br_score_lt: 0.40,
                    rpa_score_lt: 0.35,
                    risk_ge_challenge: 0.60,
                    risk_ge_deny: 0.90,
                    multi_signal_challenge: 2,
                    ..base
                },
                shadow_mode: false,
                risk_weights: (0.15, 0.25, 0.45, 0.15),
            },
            StrategyFamily::IdentityFocus => Self {
                id,
                sensitivity_sensitive: true,
                probe_depth: "standard",
                response_profile_default: ResponseProfile::Standard,
                thresholds: StrategyThresholds {
                    device_confidence_lt: 0.50,
                    os_score_lt: 0.40,
                    br_score_lt: 0.45,
                    rpa_score_lt: 0.30,
                    risk_ge_challenge: 0.60,
                    risk_ge_deny: 0.92,
                    multi_signal_challenge: 3,
                    ..base
                },
                shadow_mode: false,
                risk_weights: (0.20, 0.20, 0.15, 0.45),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectCtx {
    pub strategy: StrategyPreset,
    pub profile: ResponseProfile,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub lang: String,
    /// Public identity lane to prefer when the analyzer emitted multi-segment
    /// device IDs. This only changes projection, never minting.
    pub primary_device_lane: String,
    /// When false, still project but force device_id withheld (override emit).
    pub force_withhold_identity: bool,
}

impl Default for ProjectCtx {
    fn default() -> Self {
        let strategy = StrategyPreset::builtin(StrategyId::Balanced);
        let profile = strategy.response_profile_default;
        Self {
            strategy,
            profile,
            request_id: None,
            session_id: None,
            lang: "zh".into(),
            primary_device_lane: "dv0".into(),
            force_withhold_identity: false,
        }
    }
}

impl ProjectCtx {
    pub fn from_query(query: &Map<String, Value>) -> Self {
        let mut ctx = Self::default();
        if let Some(s) = query
            .get("strategy_id")
            .or_else(|| query.get("strategy"))
            .and_then(|v| v.as_str())
        {
            ctx.strategy = StrategyPreset::builtin(StrategyId::parse(s));
            ctx.profile = ctx.strategy.response_profile_default;
        }
        if let Some(p) = query
            .get("response_profile")
            .or_else(|| query.get("profile"))
            .and_then(|v| v.as_str())
        {
            ctx.profile = ResponseProfile::parse(p);
        }
        if let Some(r) = query.get("request_id").and_then(|v| v.as_str()) {
            ctx.request_id = Some(r.to_string());
        }
        if let Some(l) = query.get("lang").and_then(|v| v.as_str()) {
            ctx.lang = l.to_string();
        }
        if let Some(lane) = query
            .get("primary_device_lane")
            .and_then(|v| v.as_str())
            .filter(|v| matches!(*v, "dv0" | "dv4" | "dv5" | "dv6"))
        {
            ctx.primary_device_lane = lane.to_string();
        }
        ctx
    }

    pub fn from_str_map(query: &std::collections::HashMap<String, String>) -> Self {
        let mut m = Map::new();
        for (k, v) in query {
            m.insert(k.clone(), Value::String(v.clone()));
        }
        Self::from_query(&m)
    }
}

// ─── Dictionary ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct DictEntry {
    code: &'static str,
    category: &'static str,
    severity: &'static str,
    title_zh: &'static str,
    title_en: &'static str,
    description: &'static str,
    decision_hint: &'static str,
}

static SIGNAL_DICT: &[DictEntry] = &[
    // identity
    DictEntry {
        code: "identity.device_bound",
        category: "identity",
        severity: "info",
        title_zh: "设备身份已绑定",
        title_en: "Device identity bound",
        description: "已生成稳定设备 ID",
        decision_hint: "allow",
    },
    DictEntry {
        code: "identity.low_confidence",
        category: "identity",
        severity: "medium",
        title_zh: "识别置信度偏低",
        title_en: "Low identity confidence",
        description: "信号不足或不稳定，建议加强验证",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "identity.withheld",
        category: "identity",
        severity: "low",
        title_zh: "身份暂缓下发",
        title_en: "Identity withheld",
        description: "探测未完成或未达空闲门控，device_id 暂不对外",
        decision_hint: "observe",
    },
    DictEntry {
        code: "identity.collision_risk",
        category: "identity",
        severity: "medium",
        title_zh: "设备碰撞风险",
        title_en: "Collision risk",
        description: "可能与其他设备指纹相近",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "identity.ephemeral",
        category: "identity",
        severity: "low",
        title_zh: "临时设备标识",
        title_en: "Ephemeral device id",
        description: "当前为临时 ID（dve），商业绑定需谨慎",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "identity.missing_probe",
        category: "identity",
        severity: "medium",
        title_zh: "探测材料不足",
        title_en: "Insufficient probe materials",
        description: "多为网关仅或早期薄样本，需继续采集",
        decision_hint: "observe",
    },
    // automation
    DictEntry {
        code: "bot.not_detected",
        category: "automation",
        severity: "info",
        title_zh: "未检测到机器人",
        title_en: "No bot detected",
        description: "未发现自动化框架或无头特征",
        decision_hint: "allow",
    },
    DictEntry {
        code: "bot.good",
        category: "automation",
        severity: "info",
        title_zh: "善意机器人",
        title_en: "Good bot",
        description: "搜索引擎等可放行的自动化",
        decision_hint: "allow",
    },
    DictEntry {
        code: "bot.bad",
        category: "automation",
        severity: "critical",
        title_zh: "恶意机器人",
        title_en: "Bad bot",
        description: "Selenium/无头等恶意自动化特征",
        decision_hint: "deny",
    },
    DictEntry {
        code: "bot.webdriver",
        category: "automation",
        severity: "critical",
        title_zh: "WebDriver 特征",
        title_en: "WebDriver detected",
        description: "检测到 WebDriver/CDP/自动化驱动",
        decision_hint: "deny",
    },
    DictEntry {
        code: "bot.crawler",
        category: "automation",
        severity: "high",
        title_zh: "爬虫流量",
        title_en: "Crawler traffic",
        description: "UA/网关判定为爬虫",
        decision_hint: "deny",
    },
    DictEntry {
        code: "bot.rpa_high",
        category: "automation",
        severity: "high",
        title_zh: "高 RPA 行为风险",
        title_en: "High RPA risk",
        description: "轨迹更像脚本或控制面自动化",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "bot.rpa_unknown",
        category: "automation",
        severity: "low",
        title_zh: "行为证据不足",
        title_en: "Insufficient behavior evidence",
        description: "RPA 样本不够，不单独定罪",
        decision_hint: "observe",
    },
    // integrity
    DictEntry {
        code: "integrity.tampering",
        category: "integrity",
        severity: "high",
        title_zh: "指纹/环境被篡改",
        title_en: "Tampering suspected",
        description: "属性组合异常或伪装迹象",
        decision_hint: "deny",
    },
    DictEntry {
        code: "integrity.anti_detect_browser",
        category: "integrity",
        severity: "critical",
        title_zh: "反检测浏览器",
        title_en: "Anti-detect browser",
        description: "像打码/伪装浏览器环境",
        decision_hint: "deny",
    },
    DictEntry {
        code: "integrity.vm",
        category: "integrity",
        severity: "medium",
        title_zh: "虚拟机环境",
        title_en: "Virtual machine",
        description: "运行在 VM 或软环境特征明显",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "integrity.os_spoof",
        category: "integrity",
        severity: "high",
        title_zh: "操作系统声明异常",
        title_en: "OS claim anomaly",
        description: "OS 诚实度偏低，声明与观测不一致",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "integrity.br_spoof",
        category: "integrity",
        severity: "high",
        title_zh: "浏览器声明异常",
        title_en: "Browser claim anomaly",
        description: "浏览器/引擎一致性存疑",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "integrity.prototype_tamper",
        category: "integrity",
        severity: "high",
        title_zh: "原型链篡改",
        title_en: "Prototype chain tamper",
        description: "API 被 hook 或原型被改",
        decision_hint: "challenge",
    },
    // network stubs
    DictEntry {
        code: "network.unknown",
        category: "network",
        severity: "info",
        title_zh: "网络风险未知",
        title_en: "Network risk unknown",
        description: "当前无顶层 VPN/代理/机房信号（待 P1）",
        decision_hint: "observe",
    },
    DictEntry {
        code: "network.datacenter",
        category: "network",
        severity: "medium",
        title_zh: "机房/云出口 IP",
        title_en: "Datacenter IP",
        description: "IP 归属云/机房 ASN，滥用场景可加强验证",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "integrity.ja4_ua_mismatch",
        category: "integrity",
        severity: "high",
        title_zh: "TLS 指纹与 UA 不一致",
        title_en: "JA4/TLS vs User-Agent mismatch",
        description: "网关 JA4 与浏览器 UA 家族粗冲突，可能伪装或中间代理",
        decision_hint: "challenge",
    },
    DictEntry {
        code: "velocity.elevated",
        category: "velocity",
        severity: "medium",
        title_zh: "短时高频访问",
        title_en: "Elevated velocity",
        description: "同一设备/IP 在滑动窗口内请求偏多",
        decision_hint: "challenge",
    },
    // device
    DictEntry {
        code: "device.homogenized",
        category: "velocity",
        severity: "medium",
        title_zh: "环境同质化",
        title_en: "Homogenized environment",
        description: "设备/环境呈现农场同质特征",
        decision_hint: "challenge",
    },
];

fn dict_lookup(code: &str) -> Option<&'static DictEntry> {
    SIGNAL_DICT.iter().find(|e| e.code == code)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn f64_field(v: &Value, path: &str) -> Option<f64> {
    v.pointer(path).and_then(|x| x.as_f64())
}

fn str_field(v: &Value, path: &str) -> Option<String> {
    v.pointer(path)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn bool_field(v: &Value, path: &str) -> Option<bool> {
    v.pointer(path).and_then(|x| x.as_bool())
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn reasons_contain(reasons: &Value, needle: &str) -> bool {
    reasons
        .as_array()
        .map(|a| {
            a.iter().any(|r| {
                r.as_str()
                    .map(|s| s.to_ascii_lowercase().contains(needle))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn flags_contain(flags: &Value, needles: &[&str]) -> bool {
    let arr = match flags.as_array() {
        Some(a) => a,
        None => return false,
    };
    arr.iter().any(|f| {
        let s = f.as_str().unwrap_or("").to_ascii_lowercase();
        needles.iter().any(|n| s.contains(n))
    })
}

// ─── Risk ────────────────────────────────────────────────────────────────────

fn axis_score(product: &Value, axis: &str) -> f64 {
    f64_field(product, &format!("/{axis}/score")).unwrap_or(0.5)
}

fn compute_risk(product: &Value, strategy: &StrategyPreset) -> f64 {
    let (wo, wb, wr, wc) = strategy.risk_weights;
    let os = axis_score(product, "os");
    let br = axis_score(product, "br");
    let rpa = axis_score(product, "rpa");
    let conf = f64_field(product, "/device_confidence")
        .or_else(|| f64_field(product, "/confidence"))
        .unwrap_or(0.5);
    // Treat missing_probe zero-scores carefully: if all zero and status missing, risk mid
    let os_st = str_field(product, "/os/status").unwrap_or_default();
    let br_st = str_field(product, "/br/status").unwrap_or_default();
    let rpa_st = str_field(product, "/rpa/status").unwrap_or_default();
    if os_st == "missing_probe" && br_st == "missing_probe" && rpa_st == "missing_probe" {
        return 0.45; // incomplete evidence — not high bot risk, not clean
    }
    let safety = clamp01(wo * os + wb * br + wr * rpa + wc * conf);
    clamp01(1.0 - safety)
}

// ─── Bot map ─────────────────────────────────────────────────────────────────

/// Map internal bot → Fingerprint-like three-state.
fn map_bot(full: &Value, product: &Value) -> Value {
    let bot = full.get("bot").cloned().unwrap_or(json!({}));
    let verdict = bot
        .get("verdict")
        .and_then(|v| v.as_str())
        .or_else(|| product.get("bot_verdict").and_then(|v| v.as_str()))
        .unwrap_or("unknown")
        .to_ascii_lowercase();
    let flags = bot.get("flags").cloned().unwrap_or(json!([]));
    let family = bot
        .get("family")
        .or_else(|| bot.get("robot_name"))
        .cloned()
        .unwrap_or(Value::Null);
    let robot_name = bot
        .get("robot_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let (result, bot_type) = match verdict.as_str() {
        "human" | "likely_human" => ("not_detected", None),
        "crawler" => {
            // Good bots (search engines) vs generic crawler — use robot_name when present.
            let good = robot_name
                .as_ref()
                .map(|n| {
                    let l = n.to_ascii_lowercase();
                    l.contains("google")
                        || l.contains("bing")
                        || l.contains("baidu")
                        || l.contains("yandex")
                        || l.contains("duckduck")
                })
                .unwrap_or(false);
            if good {
                ("good", robot_name.clone())
            } else {
                ("bad", robot_name.or_else(|| Some("crawler".into())))
            }
        }
        "bot" | "automation" | "suspect" => {
            let t = if flags_contain(
                &flags,
                &["webdriver", "selenium", "playwright", "puppeteer", "cdc"],
            ) {
                Some("webdriver".into())
            } else {
                robot_name.or_else(|| Some(verdict.clone()))
            };
            ("bad", t)
        }
        "watch" => ("not_detected", Some("watch".into())),
        _ => ("not_detected", None),
    };

    json!({
        "result": result,
        "type": bot_type,
        "verdict": verdict,
        "flags": flags,
        "family": family,
        "score": bot.get("score"),
    })
}

// ─── Signals ─────────────────────────────────────────────────────────────────

fn make_signal(
    code: &str,
    state: &str,
    evidence: Value,
    confidence: &str,
) -> Option<Value> {
    let d = dict_lookup(code)?;
    Some(json!({
        "code": d.code,
        "state": state,
        "severity": d.severity,
        "category": d.category,
        "title_zh": d.title_zh,
        "title_en": d.title_en,
        "description": d.description,
        "confidence": confidence,
        "decision_hint": d.decision_hint,
        "evidence": evidence,
    }))
}

fn materialize_signals(full: &Value, product: &Value, bot_pub: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let conf = f64_field(product, "/device_confidence")
        .or_else(|| f64_field(product, "/confidence"))
        .unwrap_or(0.0);
    let emit = bool_field(product, "/sdk_return/emit").unwrap_or(false);
    let device_id = product
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let collision = bool_field(product, "/collision_risk").unwrap_or(false);
    let os_score = axis_score(product, "os");
    let br_score = axis_score(product, "br");
    let rpa_score = axis_score(product, "rpa");
    let os_status = str_field(product, "/os/status").unwrap_or_default();
    let br_status = str_field(product, "/br/status").unwrap_or_default();
    let rpa_status = str_field(product, "/rpa/status").unwrap_or_default();
    let rpa_bot = f64_field(product, "/rpa/bot_score").unwrap_or(0.0);
    let rpa_reasons = product
        .pointer("/rpa/reasons")
        .cloned()
        .unwrap_or(json!([]));
    let os_reasons = product
        .pointer("/os/reasons")
        .cloned()
        .unwrap_or(json!([]));
    let br_reasons = product
        .pointer("/br/reasons")
        .cloned()
        .unwrap_or(json!([]));
    let flags = bot_pub.get("flags").cloned().unwrap_or(json!([]));
    let bot_result = bot_pub
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("not_detected");
    let bot_verdict = bot_pub
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Identity signals
    if os_status == "missing_probe" || br_status == "missing_probe" {
        if let Some(s) = make_signal(
            "identity.missing_probe",
            "triggered",
            json!({"os_status": os_status, "br_status": br_status}),
            "high",
        ) {
            out.push(s);
        }
    }
    if !emit {
        if let Some(s) = make_signal(
            "identity.withheld",
            "triggered",
            json!({"sdk_return": product.get("sdk_return")}),
            "high",
        ) {
            out.push(s);
        }
    }
    if emit && !device_id.is_empty() && !device_id.starts_with("dve-") {
        if let Some(s) = make_signal(
            "identity.device_bound",
            "triggered",
            json!({"device_id_prefix": device_id.chars().take(12).collect::<String>()}),
            "high",
        ) {
            out.push(s);
        }
    }
    if device_id.starts_with("dve-") {
        if let Some(s) = make_signal(
            "identity.ephemeral",
            "triggered",
            json!({"device_id_prefix": "dve"}),
            "medium",
        ) {
            out.push(s);
        }
    }
    if conf > 0.0 && conf < 0.55 {
        if let Some(s) = make_signal(
            "identity.low_confidence",
            "triggered",
            json!({"device_confidence": conf}),
            "medium",
        ) {
            out.push(s);
        }
    }
    if collision {
        if let Some(s) = make_signal(
            "identity.collision_risk",
            "triggered",
            json!({"collision_risk": true}),
            "medium",
        ) {
            out.push(s);
        }
    }

    // Bot / automation
    if bot_result == "not_detected" && bot_verdict != "watch" {
        if let Some(s) = make_signal("bot.not_detected", "clear", json!({}), "high") {
            out.push(s);
        }
    } else if bot_result == "good" {
        if let Some(s) = make_signal(
            "bot.good",
            "triggered",
            json!({"type": bot_pub.get("type")}),
            "high",
        ) {
            out.push(s);
        }
    } else if bot_result == "bad" {
        if bot_verdict == "crawler" {
            if let Some(s) = make_signal(
                "bot.crawler",
                "triggered",
                json!({"verdict": bot_verdict}),
                "high",
            ) {
                out.push(s);
            }
        } else if let Some(s) = make_signal(
            "bot.bad",
            "triggered",
            json!({"verdict": bot_verdict, "type": bot_pub.get("type")}),
            "high",
        ) {
            out.push(s);
        }
    }
    // Also honor FE automation_globals_v2 / agent_has_* flattened into product fields
    let agent_wd = bool_field(product, "/agent_has_webdriver").unwrap_or(false)
        || bool_field(product, "/automation_globals_v2/webdriver").unwrap_or(false)
        || bool_field(product, "/fields/agent_has_webdriver").unwrap_or(false);
    let agent_auto = bool_field(product, "/agent_has_selenium").unwrap_or(false)
        || bool_field(product, "/agent_has_puppeteer").unwrap_or(false)
        || bool_field(product, "/agent_has_playwright").unwrap_or(false)
        || bool_field(product, "/agent_has_cdc").unwrap_or(false)
        || bool_field(product, "/automation_globals_v2/selenium").unwrap_or(false)
        || bool_field(product, "/automation_globals_v2/playwright").unwrap_or(false);
    if flags_contain(
        &flags,
        &["webdriver", "selenium", "playwright", "puppeteer", "cdc", "cdp"],
    ) || reasons_contain(&rpa_reasons, "webdriver")
        || reasons_contain(&rpa_reasons, "cdp_")
        || agent_wd
        || agent_auto
    {
        if let Some(s) = make_signal(
            "bot.webdriver",
            "triggered",
            json!({
                "flags": flags,
                "agent_has_webdriver": agent_wd,
                "agent_automation": agent_auto,
            }),
            "high",
        ) {
            out.push(s);
        }
    }
    // RPA: only treat as high when evidence is strong; do not over-fire on
    // rpa_status=bot alone (real traffic shows massive false positive).
    let rpa_hard = reasons_contain(&rpa_reasons, "webdriver")
        || reasons_contain(&rpa_reasons, "matrix_veto")
        || reasons_contain(&rpa_reasons, "sensitive_action_zero_move")
        || (rpa_bot >= 0.70 && rpa_score < 0.30);
    let rpa_sparse = rpa_status == "unknown"
        || rpa_status == "missing_probe"
        || reasons_contain(&rpa_reasons, "sample_quality_insufficient")
        || reasons_contain(&rpa_reasons, "behavior_early_bound");
    if rpa_hard {
        if let Some(s) = make_signal(
            "bot.rpa_high",
            "triggered",
            json!({
                "rpa_score": rpa_score,
                "rpa_bot_score": rpa_bot,
                "rpa_status": rpa_status,
            }),
            "high",
        ) {
            out.push(s);
        }
    } else if rpa_sparse || rpa_status == "automation_suspect" {
        if let Some(s) = make_signal(
            "bot.rpa_unknown",
            "triggered",
            json!({"rpa_status": rpa_status, "rpa_score": rpa_score}),
            "low",
        ) {
            out.push(s);
        }
    }

    // Integrity — never treat missing_probe zeros as spoof.
    let os_probed = os_status != "missing_probe" && os_status != "unknown";
    let br_probed = br_status != "missing_probe" && br_status != "unknown";
    if os_probed
        && (os_score < 0.35
            || os_status == "high_risk"
            || reasons_contain(&os_reasons, "soft")
            || reasons_contain(&os_reasons, "spoof")
            || reasons_contain(&os_reasons, "vm"))
    {
        if os_score < 0.35 || os_status == "high_risk" {
            if let Some(s) = make_signal(
                "integrity.os_spoof",
                "triggered",
                json!({"os_score": os_score, "os_status": os_status}),
                "medium",
            ) {
                out.push(s);
            }
        }
    }
    if br_probed
        && (br_score < 0.40
            || br_status == "suspect"
            || reasons_contain(&br_reasons, "spoof")
            || reasons_contain(&br_reasons, "antidetect"))
    {
        if br_score < 0.40 || reasons_contain(&br_reasons, "antidetect") || br_status == "suspect"
        {
            if let Some(s) = make_signal(
                "integrity.br_spoof",
                "triggered",
                json!({"br_score": br_score, "br_status": br_status}),
                "medium",
            ) {
                out.push(s);
            }
        }
    }
    if reasons_contain(&br_reasons, "antidetect")
        || reasons_contain(&rpa_reasons, "antidetect_vendor")
        || bool_field(product, "/antidetect/should_downweight").unwrap_or(false)
        || full
            .pointer("/device/antidetect/should_downweight")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        if let Some(s) = make_signal(
            "integrity.anti_detect_browser",
            "triggered",
            json!({}),
            "medium",
        ) {
            out.push(s);
        }
    }
    if reasons_contain(&os_reasons, "vm")
        || reasons_contain(&br_reasons, "vm")
        || str_field(product, "/association_level")
            .map(|l| l == "env" || l == "profile")
            .unwrap_or(false)
    {
        if let Some(s) = make_signal(
            "integrity.vm",
            "triggered",
            json!({"association_level": product.get("association_level")}),
            "medium",
        ) {
            out.push(s);
        }
    }
    if reasons_contain(&br_reasons, "prototype")
        || reasons_contain(&os_reasons, "prototype")
        || bool_field(product, "/prototype_chain_tamper").unwrap_or(false)
    {
        if let Some(s) = make_signal("integrity.prototype_tamper", "triggered", json!({}), "high")
        {
            out.push(s);
        }
    }
    // Tampering aggregate (probed only)
    if (os_probed && br_probed && os_score < 0.32 && br_score < 0.32)
        || reasons_contain(&br_reasons, "tamper")
        || reasons_contain(&os_reasons, "spoof_score")
    {
        if let Some(s) = make_signal(
            "integrity.tampering",
            "triggered",
            json!({"os_score": os_score, "br_score": br_score}),
            "high",
        ) {
            out.push(s);
        }
    }

    // Homogenization
    let homo_sev = product
        .pointer("/homogenization/severity")
        .or_else(|| product.pointer("/environment_homogeneity/severity"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(homo_sev, "high" | "critical" | "hot")
        || bool_field(product, "/homogenization/supercluster").unwrap_or(false)
    {
        if let Some(s) = make_signal(
            "device.homogenized",
            "triggered",
            json!({"homogenization": product.get("homogenization")}),
            "medium",
        ) {
            out.push(s);
        }
    }

    // JA4 (Pingora) ↔ User-Agent family mismatch
    let ja4 = str_field(product, "/ja4")
        .or_else(|| str_field(product, "/tls_ja4"))
        .or_else(|| {
            full
                .pointer("/fields/ja4")
                .or_else(|| full.pointer("/fields/tls_ja4"))
                .or_else(|| full.pointer("/gateway_fields/ja4"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let ua = str_field(product, "/user_agent").or_else(|| {
        full
            .pointer("/fields/user_agent")
            .or_else(|| full.get("user_agent"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    let pe = str_field(product, "/protocol_engine").or_else(|| {
        full
            .pointer("/fields/protocol_engine")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    let alpn = str_field(product, "/alpn").or_else(|| {
        full
            .pointer("/fields/alpn")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    if let Some(hint) = crate::net_enrich::ja4_ua_mismatch_ex(
        ja4.as_deref(),
        ua.as_deref(),
        pe.as_deref(),
        alpn.as_deref(),
    ) {
        if let Some(s) = make_signal(
            "integrity.ja4_ua_mismatch",
            "triggered",
            json!({
                "hint": hint,
                "ja4": ja4,
                "protocol_engine": pe,
                "ua_prefix": ua.as_ref().map(|u| u.chars().take(48).collect::<String>())
            }),
            "medium",
        ) {
            out.push(s);
        }
    }

    // Network denorm (ASN / datacenter) — product or fields (gateway B8)
    let asn_v = product
        .get("server_asn")
        .or_else(|| full.pointer("/fields/server_asn"))
        .or_else(|| full.pointer("/gateway_fields/server_asn"))
        .cloned();
    let country_v = product
        .get("server_country")
        .or_else(|| full.pointer("/fields/server_country"))
        .or_else(|| full.pointer("/gateway_fields/server_country"))
        .cloned();
    let has_dc = bool_field(product, "/server_datacenter").unwrap_or(false)
        || str_field(product, "/server_network_class")
            .map(|c| c.contains("datacenter"))
            .unwrap_or(false)
        || full
            .pointer("/fields/server_datacenter")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || full
            .pointer("/fields/server_network_class")
            .and_then(|v| v.as_str())
            .map(|c| c.contains("datacenter"))
            .unwrap_or(false);
    if has_dc {
        if let Some(s) = make_signal(
            "network.datacenter",
            "triggered",
            json!({
                "asn": asn_v,
                "country": country_v,
                "class": product.get("server_network_class")
                    .or_else(|| full.pointer("/fields/server_network_class")),
            }),
            "medium",
        ) {
            out.push(s);
        }
    }
    if let Some(v) = f64_field(product, "/velocity_score")
        .or_else(|| {
            full
                .pointer("/product/velocity_score")
                .and_then(|x| x.as_f64())
        })
        .or_else(|| {
            full
                .pointer("/velocity/velocity_score")
                .and_then(|x| x.as_f64())
        })
    {
        if v >= 0.7 {
            if let Some(s) = make_signal(
                "velocity.elevated",
                "triggered",
                json!({
                    "velocity_score": v,
                    "windows": product.get("velocity_windows")
                        .or_else(|| full.pointer("/product/velocity_windows"))
                        .or_else(|| full.pointer("/velocity")),
                }),
                "medium",
            ) {
                out.push(s);
            }
        }
    }

    // Network stub when no datacenter / ASN materials
    let has_net = has_dc
        || asn_v
            .as_ref()
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
    if !has_net {
        if let Some(s) = make_signal("network.unknown", "unknown", json!({}), "low") {
            out.push(s);
        }
    }

    out
}

// ─── Public action ───────────────────────────────────────────────────────────

fn signal_triggered(signals: &[Value], code: &str) -> bool {
    signals.iter().any(|s| {
        s.get("code").and_then(|c| c.as_str()) == Some(code)
            && s.get("state").and_then(|st| st.as_str()) == Some("triggered")
    })
}

fn count_high_triggered(signals: &[Value]) -> u32 {
    signals
        .iter()
        .filter(|s| {
            s.get("state").and_then(|st| st.as_str()) == Some("triggered")
                && matches!(
                    s.get("severity").and_then(|x| x.as_str()),
                    Some("high") | Some("critical")
                )
        })
        .count() as u32
}

fn map_legacy_action(legacy: &str) -> PublicAction {
    match legacy {
        "allow" => PublicAction::Allow,
        "allow_soft" | "step_up" => PublicAction::Challenge,
        "challenge_bot" | "deny_sensitive" => PublicAction::Deny,
        _ => PublicAction::Allow,
    }
}

fn decide_public_action(
    product: &Value,
    bot_pub: &Value,
    signals: &[Value],
    strategy: &StrategyPreset,
    risk: f64,
) -> (PublicAction, Vec<String>, Option<String>) {
    let mut reasons = Vec::new();
    let thr = &strategy.thresholds;
    let conf = f64_field(product, "/device_confidence")
        .or_else(|| f64_field(product, "/confidence"))
        .unwrap_or(0.5);
    let os = axis_score(product, "os");
    let br = axis_score(product, "br");
    let rpa = axis_score(product, "rpa");
    let bot_result = bot_pub
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("not_detected");
    let legacy = product
        .pointer("/recommended_action/action")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Observe-only / shadow: never hard block
    if strategy.shadow_mode || strategy.id == StrategyId::ObserveOnly {
        let would = decide_public_action_inner(
            product,
            bot_pub,
            signals,
            strategy,
            risk,
            conf,
            os,
            br,
            rpa,
            bot_result,
            legacy,
            &mut reasons,
        );
        reasons.insert(0, "observe_only_shadow".into());
        return (
            PublicAction::Allow,
            reasons,
            Some(would.as_str().to_string()),
        );
    }

    let action = decide_public_action_inner(
        product,
        bot_pub,
        signals,
        strategy,
        risk,
        conf,
        os,
        br,
        rpa,
        bot_result,
        legacy,
        &mut reasons,
    );
    let _ = thr;
    (action, reasons, None)
}

fn decide_public_action_inner(
    product: &Value,
    bot_pub: &Value,
    signals: &[Value],
    strategy: &StrategyPreset,
    risk: f64,
    conf: f64,
    os: f64,
    br: f64,
    rpa: f64,
    bot_result: &str,
    legacy: &str,
    reasons: &mut Vec<String>,
) -> PublicAction {
    let thr = &strategy.thresholds;
    let fam = strategy.id.family();
    let bot_verdict = bot_pub
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let is_crawler =
        signal_triggered(signals, "bot.crawler") || bot_verdict.eq_ignore_ascii_case("crawler");

    // ── L1 hard automation (G4) — all non-observe strategies ──────────────
    if signal_triggered(signals, "bot.webdriver") {
        reasons.push("signal:bot.webdriver".into());
        return PublicAction::Deny;
    }

    // Malicious automation (not crawler-only): deny on most strategies
    if signal_triggered(signals, "bot.bad") && !is_crawler {
        reasons.push("signal:bot.bad".into());
        return PublicAction::Deny;
    }

    // Crawler (G5): hard deny only on bot_control family; else challenge
    if is_crawler {
        if fam == StrategyFamily::BotControl {
            reasons.push("signal:bot.crawler".into());
            return PublicAction::Deny;
        }
        // balanced/login/identity: challenge (collect / soft block), not deny
        reasons.push("crawler_challenge".into());
        return PublicAction::Challenge;
    }

    // AD / fingerprint browser (G3)
    if signal_triggered(signals, "integrity.anti_detect_browser") {
        match fam {
            StrategyFamily::IdentityFocus => {
                reasons.push("signal:integrity.anti_detect_browser".into());
                // identity_focus: challenge unless multi-signal later escalates
            }
            StrategyFamily::BotControl => {
                // bot_control focuses automation; still treat AD as challenge
                reasons.push("signal:integrity.anti_detect_browser".into());
            }
            _ => {
                reasons.push("signal:integrity.anti_detect_browser".into());
                return PublicAction::Deny;
            }
        }
    }
    if signal_triggered(signals, "integrity.tampering")
        && matches!(
            fam,
            StrategyFamily::StrictIntegrity | StrategyFamily::Register
        )
    {
        reasons.push("signal:integrity.tampering".into());
        return PublicAction::Deny;
    }

    // bot.result=bad for non-crawler (crawler handled above)
    if bot_result == "bad" && !is_crawler {
        reasons.push(format!("bot.result={bot_result}"));
        return PublicAction::Deny;
    }

    // Risk thresholds (do not deny on soft RPA alone — risk fusion weights rpa carefully)
    if risk >= thr.risk_ge_deny {
        // Guard: pure soft-rpa / collision must not sole-deny
        let only_soft = signal_triggered(signals, "bot.rpa_unknown")
            && !signal_triggered(signals, "bot.webdriver")
            && !signal_triggered(signals, "bot.bad")
            && !signal_triggered(signals, "integrity.anti_detect_browser")
            && !signal_triggered(signals, "integrity.os_spoof");
        if only_soft {
            reasons.push(format!(
                "risk_ge_deny_soft_guard:{risk:.3}→challenge"
            ));
            return PublicAction::Challenge;
        }
        reasons.push(format!("risk_ge_deny:{risk:.3}>={}", thr.risk_ge_deny));
        return PublicAction::Deny;
    }

    // ── Challenge conditions (G1/G2/G4 soft / G6) ─────────────────────────
    let mut challenge = signal_triggered(signals, "integrity.anti_detect_browser")
        && fam == StrategyFamily::IdentityFocus;

    if strategy.sensitivity_sensitive && conf < thr.device_confidence_lt {
        reasons.push(format!(
            "sensitive_low_conf:{conf:.3}<{}",
            thr.device_confidence_lt
        ));
        challenge = true;
    }
    let os_st = str_field(product, "/os/status").unwrap_or_default();
    let br_st = str_field(product, "/br/status").unwrap_or_default();
    let os_probed = os_st != "missing_probe" && os_st != "unknown";
    let br_probed = br_st != "missing_probe" && br_st != "unknown";
    if os_probed && os < thr.os_score_lt {
        reasons.push(format!("os_score_lt:{os:.3}"));
        challenge = true;
    }
    if br_probed && br < thr.br_score_lt {
        reasons.push(format!("br_score_lt:{br:.3}"));
        challenge = true;
    }
    // RPA: challenge on hard rpa_high only — never deny on soft rpa_unknown alone
    if signal_triggered(signals, "bot.rpa_high") {
        reasons.push(format!("rpa_challenge:{rpa:.3}"));
        challenge = true;
    } else if rpa < thr.rpa_score_lt
        && rpa > 0.0
        && !signal_triggered(signals, "bot.rpa_unknown")
        && !signal_triggered(signals, "identity.missing_probe")
    {
        reasons.push(format!("rpa_score_lt:{rpa:.3}"));
        challenge = true;
    }
    if signal_triggered(signals, "integrity.os_spoof")
        || signal_triggered(signals, "integrity.br_spoof")
        || signal_triggered(signals, "integrity.vm")
    {
        reasons.push("integrity_challenge".into());
        challenge = true;
        // Strict integrity: escalate high_risk OS to deny when multi-signal or very low score
        if fam == StrategyFamily::StrictIntegrity
            && signal_triggered(signals, "integrity.os_spoof")
            && (os < 0.20 || count_high_triggered(signals) >= 2)
        {
            reasons.push("strict_integrity_os_deny".into());
            return PublicAction::Deny;
        }
    }
    if signal_triggered(signals, "identity.collision_risk") && strategy.sensitivity_sensitive {
        // collision alone is challenge never deny (178: ~78% true)
        reasons.push("collision_risk_sensitive".into());
        challenge = true;
    }
    if signal_triggered(signals, "device.homogenized")
        && matches!(
            fam,
            StrategyFamily::Register | StrategyFamily::StrictIntegrity
        )
    {
        reasons.push("homogenized_farm".into());
        challenge = true;
    }
    let high_n = count_high_triggered(signals);
    if high_n >= thr.multi_signal_challenge {
        reasons.push(format!("multi_high_signals:{high_n}"));
        challenge = true;
    }
    if risk >= thr.risk_ge_challenge {
        reasons.push(format!("risk_ge_challenge:{risk:.3}"));
        challenge = true;
    }
    // Missing probe → challenge / collect_more (never forge bot deny)
    if signal_triggered(signals, "identity.missing_probe") {
        reasons.push("missing_probe_collect_more".into());
        challenge = true;
    }

    if challenge {
        return PublicAction::Challenge;
    }

    // Fold legacy recommended_action if present and stricter — but never escalate
    // soft rpa overfire via legacy challenge_bot alone without L1 evidence
    if !legacy.is_empty() {
        let mapped = map_legacy_action(legacy);
        if mapped == PublicAction::Deny {
            if signal_triggered(signals, "bot.webdriver")
                || signal_triggered(signals, "bot.bad")
                || bot_result == "bad"
            {
                reasons.push(format!("legacy_action={legacy}"));
                return PublicAction::Deny;
            }
            // legacy deny without hard evidence → challenge only
            reasons.push(format!("legacy_action={legacy}_softened"));
            return PublicAction::Challenge;
        }
        if mapped == PublicAction::Challenge {
            reasons.push(format!("legacy_action={legacy}"));
            return PublicAction::Challenge;
        }
    }

    if reasons.is_empty() {
        reasons.push("default_allow".into());
    }
    let _ = (product, bot_pub);
    PublicAction::Allow
}

// ─── Profile trim ────────────────────────────────────────────────────────────

fn trim_profile(out: &mut Value, profile: ResponseProfile) {
    let obj = match out.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    match profile {
        ResponseProfile::Basic => {
            // Keep top signals only (≤5 triggered preferred)
            if let Some(signals) = obj.get_mut("signals").and_then(|s| s.as_array_mut()) {
                // Prefer triggered, then cap 5
                signals.sort_by_key(|s| {
                    let st = s.get("state").and_then(|x| x.as_str()).unwrap_or("");
                    if st == "triggered" {
                        0
                    } else {
                        1
                    }
                });
                signals.truncate(5);
                for s in signals.iter_mut() {
                    if let Some(o) = s.as_object_mut() {
                        o.insert("evidence".into(), json!({}));
                    }
                }
            }
            // Keep compact network/velocity (ASN/class/score) — G1–G6 decision inputs.
            // Drop only heavy diagnostics / identity segment arrays.
            obj.insert("diagnostics".into(), Value::Null);
            // Drop segments detail
            if let Some(id) = obj.get_mut("identity").and_then(|i| i.as_object_mut()) {
                id.remove("device_id_segments");
                id.remove("parts_present");
            }
        }
        ResponseProfile::Standard => {
            if let Some(signals) = obj.get_mut("signals").and_then(|s| s.as_array_mut()) {
                for s in signals.iter_mut() {
                    // Keep compact evidence only
                    if let Some(o) = s.as_object_mut() {
                        if let Some(ev) = o.get("evidence").cloned() {
                            // drop large blobs
                            if ev.to_string().len() > 400 {
                                o.insert("evidence".into(), json!({"truncated": true}));
                            }
                        }
                    }
                }
            }
            obj.insert("diagnostics".into(), Value::Null);
        }
        ResponseProfile::Advanced => {
            obj.insert("diagnostics".into(), Value::Null);
        }
        ResponseProfile::Diagnostic => {
            // keep diagnostics if present
        }
    }
}

// ─── Main project ────────────────────────────────────────────────────────────

/// Project full evaluate result (or product-only) into merchant envelope.
pub fn project(full: &Value, ctx: &ProjectCtx) -> Value {
    let product = full
        .get("product")
        .cloned()
        .unwrap_or_else(|| full.clone());
    let session_id = ctx
        .session_id
        .clone()
        .or_else(|| str_field(full, "/session_id"))
        .or_else(|| str_field(full, "/cycle_id"));

    let emit = if ctx.force_withhold_identity {
        false
    } else {
        bool_field(&product, "/sdk_return/emit").unwrap_or(true)
        // default true for offline packaging of historical analysis without gate stamp
    };

    let device_id_raw = product
        .get("device_id")
        .cloned()
        .unwrap_or(Value::Null);
    let device_id = if emit {
        product
            .get("device_id_segments")
            .and_then(|v| v.get(&ctx.primary_device_lane))
            .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
            .cloned()
            .unwrap_or_else(|| device_id_raw.clone())
    } else {
        Value::Null
    };

    let conf = f64_field(&product, "/device_confidence")
        .or_else(|| f64_field(&product, "/confidence"))
        .unwrap_or(0.0);

    let bot_pub = map_bot(full, &product);
    let risk = compute_risk(&product, &ctx.strategy);
    let suspect = (risk * 100.0).round() as i64;

    let mut signals = materialize_signals(full, &product, &bot_pub);
    let use_en = ctx.lang.trim().to_ascii_lowercase().starts_with("en");
    for signal in &mut signals {
        let code = signal.get("code").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(dict) = dict_lookup(code) {
            if let Some(obj) = signal.as_object_mut() {
                obj.insert(
                    "title".into(),
                    json!(if use_en { dict.title_en } else { dict.title_zh }),
                );
                obj.insert("description_localized".into(), json!(dict.description));
            }
        }
    }
    let (action, action_reasons, would) =
        decide_public_action(&product, &bot_pub, &signals, &ctx.strategy, risk);

    // Attach strategy_id to meta
    let mut action_obj = json!({
        "action": action.as_str(),
        "reasons": action_reasons,
        "schema": "gr_recommended_action_v2",
        "strategy_id": ctx.strategy.id.as_str(),
        "legacy_action": product.get("recommended_action"),
    });
    if let Some(w) = would {
        if let Some(o) = action_obj.as_object_mut() {
            o.insert("would_have_action".into(), json!(w));
        }
    }

    let scores = json!({
        "risk": risk,
        "suspect_score": suspect,
        "os": {
            "score": product.pointer("/os/score"),
            "status": product.pointer("/os/status"),
            "coverage": product.pointer("/os/coverage"),
        },
        "br": {
            "score": product.pointer("/br/score"),
            "status": product.pointer("/br/status"),
            "coverage": product.pointer("/br/coverage"),
        },
        "rpa": {
            "score": product.pointer("/rpa/score"),
            "status": product.pointer("/rpa/status"),
            "coverage": product.pointer("/rpa/coverage"),
            "human_score": product.pointer("/rpa/human_score"),
            "bot_score": product.pointer("/rpa/bot_score"),
        },
    });

    // parts_present from segments if available
    let segments = product.get("device_id_segments").cloned().unwrap_or(Value::Null);
    let parts_present = segments
        .as_object()
        .map(|m| {
            m.values()
                .filter(|v| {
                    v.as_str()
                        .map(|s| !s.is_empty() && s != "0" && s != "null")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);

    // iss/opus5 01-P0-1: layered identity for the SDK — class id (cross-network
    // stable, NOT machine-unique) + instance id (H(class‖separator), withheld
    // when no stable separator), each with its own confidence and semantics.
    let device_class_id = if emit {
        product.get("device_class_id").cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let device_instance_id = if emit {
        product.get("device_instance_id").cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let identity = json!({
        "device_id": device_id,
        "device_id_segments": if matches!(ctx.profile, ResponseProfile::Advanced | ResponseProfile::Diagnostic) {
            segments
        } else {
            Value::Null
        },
        "confidence": conf,
        "tier": product.get("device_tier"),
        "seen_before": Value::Null,
        "parts_present": parts_present,
        "parts_total": 10,
        "emit": emit,
        "multi_segment": product.get("multi_segment"),
        "association_level": product.get("association_level"),
        "digest_path": product.get("digest_path"),
        // Layered IDs (gr_layered_id_v1). instance id is null when not issued.
        "device_class_id": device_class_id,
        "device_class_semantics": "device_class_config_not_machine_unique",
        "device_class_confidence": conf,
        "device_instance_id": device_instance_id,
        "device_instance_semantics": "machine_instance_hash_class_plus_separator_net_bound",
        "device_instance_confidence": product
            .get("device_instance_confidence")
            .cloned()
            .unwrap_or(Value::Null),
        "primary_device_lane": ctx.primary_device_lane.clone(),
        "identity_schema": product
            .get("identity_schema")
            .cloned()
            .unwrap_or(Value::Null),
        // iss/opus5 03-P0-2: confidence is NOT a calibrated probability until
        // the labeled-set ECE gate adopts a calibrated version — label honestly.
        "confidence_calibrated": crate::conf_cal::active_confidence_version() != "heuristic_v0",
        "confidence_algorithm": crate::conf_cal::active_confidence_version(),
    });

    let client = json!({
        "sdk": "js",
        "browser_name": product
            .pointer("/env/browser_name")
            .or_else(|| full.pointer("/truth/browser_name"))
            .cloned()
            .unwrap_or(Value::Null),
        "os": product
            .pointer("/env/os_name")
            .or_else(|| full.pointer("/truth/os_family"))
            .or_else(|| product.get("os_family"))
            .cloned()
            .unwrap_or(Value::Null),
        "form_class": product
            .get("form_class")
            .or_else(|| full.pointer("/truth/form_class"))
            .cloned()
            .unwrap_or(Value::Null),
    });

    let asn_out = product
        .get("server_asn")
        .or_else(|| full.pointer("/fields/server_asn"))
        .or_else(|| full.pointer("/gateway_fields/server_asn"))
        .cloned()
        .unwrap_or(Value::Null);
    let country_out = product
        .get("server_country")
        .or_else(|| full.pointer("/fields/server_country"))
        .or_else(|| full.pointer("/gateway_fields/server_country"))
        .or_else(|| full.pointer("/fields/cf_ipcountry"))
        .or_else(|| full.pointer("/cf_fields/country"))
        .cloned()
        .unwrap_or(Value::Null);
    let dc_out = product
        .get("server_datacenter")
        .or_else(|| full.pointer("/fields/server_datacenter"))
        .cloned()
        .unwrap_or(Value::Null);
    let net_class = product
        .get("server_network_class")
        .or_else(|| full.pointer("/fields/server_network_class"))
        .cloned()
        .unwrap_or(Value::Null);
    let ip_out = product
        .get("server_client_ip")
        .or_else(|| full.pointer("/fields/server_client_ip"))
        .or_else(|| full.get("client_ip"))
        .cloned()
        .unwrap_or(Value::Null);
    let network = json!({
        "ip": ip_out,
        "asn": asn_out,
        "country": country_out,
        "datacenter": dc_out,
        "network_class": net_class,
        "asn_org": product.get("server_asn_org")
            .or_else(|| full.pointer("/fields/server_asn_org"))
            .cloned()
            .unwrap_or(Value::Null),
        // Admin-panel third-party IP integrations (when configured) land here.
        "ip_info": product.get("ip_info")
            .or_else(|| full.pointer("/fields/ip_info"))
            .or_else(|| full.pointer("/gateway_fields/ip_info"))
            .cloned()
            .unwrap_or(Value::Null),
        "ip_reputation": product.get("ip_reputation")
            .or_else(|| full.pointer("/fields/ip_reputation"))
            .or_else(|| full.pointer("/gateway_fields/ip_reputation"))
            .cloned()
            .unwrap_or(Value::Null),
        "ip_enrichment_source": product.get("ip_enrichment_source")
            .or_else(|| product.get("server_asn_source"))
            .or_else(|| full.pointer("/fields/server_asn_source"))
            .cloned()
            .unwrap_or(Value::Null),
        "vpn": product.get("vpn")
            .or_else(|| full.pointer("/fields/vpn"))
            .cloned()
            .unwrap_or(json!({ "result": null, "note": "configure_admin_integrations" })),
        "proxy": product.get("proxy")
            .or_else(|| full.pointer("/fields/proxy"))
            .cloned()
            .unwrap_or(json!({ "result": null, "note": "configure_admin_integrations" })),
        "tor": product.get("tor")
            .or_else(|| full.pointer("/fields/tor"))
            .cloned()
            .unwrap_or(json!({ "result": null, "note": "configure_admin_integrations" })),
        // M2: gateway cross-layer coherence (sec-ch triple / platform pair /
        // TLS-H2-QUIC-TCP family vote / UA-JA4 engine) — server-side only.
        "gateway_coherence_score": full
            .pointer("/fields/gateway_coherence/score")
            .or_else(|| product.pointer("/gateway_coherence/score"))
            .cloned()
            .unwrap_or(Value::Null),
        "gateway_coherence_verdict": full
            .pointer("/fields/gateway_coherence/verdict")
            .or_else(|| product.pointer("/gateway_coherence/verdict"))
            .cloned()
            .unwrap_or(Value::Null),
        "gateway_protocol_coherence": full
            .pointer("/fields/gateway_coherence/components/protocol_cross_layer")
            .cloned()
            .unwrap_or(Value::Null),
    });

    let velocity = {
        let score = product
            .get("velocity_score")
            .or_else(|| full.pointer("/product/velocity_score"))
            .cloned();
        let windows = product
            .get("velocity_windows")
            .or_else(|| full.pointer("/product/velocity_windows"))
            .cloned();
        if score.is_some() || windows.is_some() {
            json!({
                "score": score.unwrap_or(Value::Null),
                "windows": windows.unwrap_or(Value::Null),
            })
        } else {
            Value::Null
        }
    };

    let diagnostics = if matches!(ctx.profile, ResponseProfile::Diagnostic) {
        json!({
            "real_band": full.get("real_band"),
            "sub_algorithms": product.get("sub_algorithms"),
            "analysis_quality": product.get("analysis_quality"),
            "signal_count": signals.len(),
        })
    } else {
        Value::Null
    };

    // ─── State interpretability (§4.2): five-axis coverage states ─────────────
    // Brain planes_cover (S0 edge / S1 identity / S2 multisource / S3 behavior)
    // reaches the merchant envelope so an axis without a score/status can still
    // answer "who observed what": plane state + the explaining missing batch.
    let planes_cover = full
        .get("coverage")
        .and_then(|c| c.get("planes_cover"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let cover_state = |plane: &str| -> String {
        planes_cover
            .get(plane)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let missing_state_of = |pid: &str| -> Value {
        full.get("coverage")
            .and_then(|c| c.get("missing_states").and_then(|m| m.get(pid)))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let axis_cover = |plane: &str, pids: &[&str]| -> Value {
        let state = cover_state(plane);
        if state == "not_observed" {
            for pid in pids {
                let ms = missing_state_of(pid);
                if !ms.is_null() {
                    return json!({
                        "plane": plane,
                        "state": state,
                        "missing_batch": pid,
                        "missing_state": ms.get("missing_state"),
                        "gate": ms.get("gate"),
                    });
                }
            }
            return json!({
                "plane": plane,
                "state": state,
                "missing_batch": pids.first(),
                "missing_state": Value::Null,
            });
        }
        json!({
            "plane": plane,
            "state": state,
            "missing_batch": Value::Null,
            "missing_state": Value::Null,
        })
    };
    // Axis status falls through to the plane state when the axis itself is
    // unrated (unknown/missing_probe/not_observed) — full-chain propagation.
    let axis_status = |cur: Value, plane: &str, pids: &[&str]| -> Value {
        if let Some(s) = cur.as_str() {
            if !matches!(s, "" | "unknown" | "missing_probe" | "not_observed") {
                return cur;
            }
        }
        let state = cover_state(plane);
        if state == "not_observed" {
            for pid in pids {
                let ms = missing_state_of(pid);
                if !ms.is_null() {
                    if let Some(m) = ms.get("missing_state").and_then(|v| v.as_str()) {
                        if m != "not_observed" {
                            return json!(m);
                        }
                    }
                }
            }
            json!("not_observed")
        } else {
            cur
        }
    };

    // B4/M1 blend: effective spoof risk floors at 1 - realm coherence when the
    // numeric-level realm agreement drops below 0.6; otherwise mirrors the
    // stack-auth spoof score. (Hoisted out of json! — bare blocks parse as
    // object literals inside the macro.)
    let spoof_risk_effective = {
        let auth = full
            .pointer("/device/stack_auth/spoof_score")
            .or_else(|| product.pointer("/stack_auth/spoof_score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let coh = full
            .pointer("/realm_conflict_graph/realm_coherence_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(f64::NAN);
        if coh.is_finite() && coh < 0.6 {
            json!((auth.max(1.0 - coh) * 10000.0).round() / 10000.0)
        } else {
            json!((auth * 10000.0).round() / 10000.0)
        }
    };

    let mut out = json!({
        "api_version": API_VERSION,
        "request_id": ctx.request_id,
        "session_id": session_id,
        "meta": {
            "algo_projection": PRODUCT_PUBLIC_ALGO,
            "strategy_id": ctx.strategy.id.as_str(),
            "strategy_label_zh": ctx.strategy.id.label_zh(),
            "strategy_version": 1,
            "response_profile": ctx.profile.as_str(),
            "lang": ctx.lang.clone(),
            "primary_device_lane": ctx.primary_device_lane.clone(),
            "probe_depth": ctx.strategy.probe_depth,
            "mint_algo": "unchanged",
            "emit_identity": emit,
            "product_version": full.get("product_version")
                .or_else(|| product.get("product_version"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        "identity": identity,
        "scores": scores,
        "bot": bot_pub,
        "recommended_action": action_obj,
        "signals": signals,
        "network": network,
        "client": client,
        "velocity": velocity,
        "diagnostics": diagnostics,
        "authenticity": {
            "real_band": full.get("real_band")
                .or_else(|| product.get("real_band"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        "capability_axes": {
            "browser_realism": {
                "score": product.pointer("/br/score"),
                "confidence": product.pointer("/br/coverage"),
                "coverage": product.pointer("/br/coverage"),
                "status": axis_status(
                    product.pointer("/br/status").cloned().unwrap_or(json!("unknown")),
                    "S1_identity",
                    &["B10_hw_curves", "B0_bootstrap"],
                ),
                "axis_cover": axis_cover("S1_identity", &["B10_hw_curves", "B0_bootstrap"]),
            },
            "automation_risk": {
                "score": product.pointer("/rpa/bot_score").or_else(|| product.pointer("/rpa/score")),
                "confidence": product.pointer("/rpa/coverage"),
                "coverage": product.pointer("/rpa/coverage"),
                "status": axis_status(
                    product.pointer("/rpa/status").cloned().unwrap_or(json!("unknown")),
                    "S3_behavior",
                    &["B11_interaction"],
                ),
                "axis_cover": axis_cover("S3_behavior", &["B11_interaction"]),
            },
            "spoof_risk": {
                // iss/opus5 §4.1: this axis used to mirror the bot score,
                // duplicating automation_risk. It now reflects the
                // server-derived stack-auth spoof score (GPU label vs
                // measured rendering class), with provenance annotated.
                // iss/74 M1+B4: realm coherence + xsrc numeric divergence ride
                // the same channel as side inputs (blend never replaces score).
                "score": full
                    .pointer("/device/stack_auth/spoof_score")
                    .or_else(|| product.pointer("/stack_auth/spoof_score"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "derived_from": "stack_auth.spoof_score",
                "confidence": product.get("device_confidence"),
                "coverage": product.pointer("/os/coverage"),
                "status": axis_status(
                    match full
                        .pointer("/device/stack_auth/spoof_score")
                        .or_else(|| product.pointer("/stack_auth/spoof_score"))
                        .and_then(|v| v.as_f64())
                    {
                        Some(s) if s >= 0.6 => json!("high_risk"),
                        Some(s) if s >= 0.3 => json!("suspect"),
                        Some(_) => json!("real"),
                        None => json!("unknown"),
                    },
                    "S2_multisource",
                    &["B7_sandbox"],
                ),
                "axis_cover": axis_cover("S2_multisource", &["B7_sandbox"]),
                // M1: numeric-level realm coherence (structured_realm_diff).
                "realm_coherence_score": full
                    .pointer("/realm_conflict_graph/realm_coherence_score")
                    .or_else(|| product.get("realm_coherence_score"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "realm_coherence_verdict": full
                    .pointer("/realm_conflict_graph/realm_coherence_verdict")
                    .or_else(|| product.get("realm_coherence_verdict"))
                    .cloned()
                    .unwrap_or(Value::Null),
                // B4: cross-source numeric consistency verdict (A3 machinery).
                "xsrc_numeric_verdict": full
                    .pointer("/multi_source_consistency/xsrc_numeric/xsrc_numeric_verdict")
                    .cloned()
                    .unwrap_or(Value::Null),
                // Blend: when realm coherence drops below 0.6 the effective spoof
                // risk floors at 1 - coherence; otherwise equals the auth score.
                "spoof_risk_effective": spoof_risk_effective,
            },
            "multi_account_risk": {
                "score": product.pointer("/peer_similarity/score"),
                "confidence": product.pointer("/peer_similarity/coverage"),
                "coverage": product.pointer("/peer_similarity/coverage"),
                "status": axis_status(
                    product.pointer("/peer_similarity/status").cloned().unwrap_or(json!("unknown")),
                    "S2_multisource",
                    &["B7_sandbox"],
                ),
                "axis_cover": axis_cover("S2_multisource", &["B7_sandbox"]),
            },
            "network_risk": {
                "score": product
                    .get("network_risk")
                    .and_then(|v| {
                        if v.is_number() {
                            Some(v.clone())
                        } else {
                            v.get("score").filter(|s| s.is_number()).cloned()
                        }
                    })
                    .unwrap_or(Value::Null),
                "confidence": json!(if country_out.is_null() { 0.0 } else { 0.7 }),
                "coverage": json!(if country_out.is_null() { "not_observed" } else { "observed" }),
                "status": axis_status(
                    json!(if country_out.is_null() { "not_observed" } else { "observed" }),
                    "S0_edge",
                    &["B8_gateway"],
                ),
                "axis_cover": axis_cover("S0_edge", &["B8_gateway"]),
            },
        },
        "business_context": product.get("business_context").cloned().unwrap_or(Value::Null),
        "schema_version": "product_public_v1",
    });

    // Profile may drop signal entries — re-read after
    let _ = &mut signals;
    trim_profile(&mut out, ctx.profile);
    out
}

/// Convenience: project with strategy_id / profile string args.
pub fn project_with(
    full: &Value,
    strategy_id: &str,
    response_profile: &str,
    session_id: Option<&str>,
) -> Value {
    let mut ctx = ProjectCtx {
        strategy: StrategyPreset::builtin(StrategyId::parse(strategy_id)),
        profile: ResponseProfile::parse(response_profile),
        request_id: None,
        session_id: session_id.map(|s| s.to_string()),
        lang: "zh".into(),
        primary_device_lane: "dv0".into(),
        force_withhold_identity: false,
    };
    // If profile not explicitly advanced, use strategy default when profile=standard default
    if response_profile.is_empty() {
        ctx.profile = ctx.strategy.response_profile_default;
    }
    project(full, &ctx)
}

/// All built-in strategy ids (for admin UI / docs).
/// Primary goal-intensity ids first; legacy business aliases listed after.
pub fn list_strategy_presets() -> Value {
    let ids = [
        StrategyId::ObserveOnly,
        StrategyId::Balanced,
        StrategyId::StrictIntegrity,
        StrategyId::BotControl,
        StrategyId::IdentityFocus,
        StrategyId::RegisterAntiAbuse,
        // API aliases (same families)
        StrategyId::LoginStrict,
        StrategyId::Checkout,
        StrategyId::ContentBot,
    ];
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            let p = StrategyPreset::builtin(*id);
            json!({
                "strategy_id": id.as_str(),
                "label_en": id.label_en(),
                "label_zh": id.label_zh(),
                "family": match id.family() {
                    StrategyFamily::Observe => "observe",
                    StrategyFamily::Balanced => "balanced",
                    StrategyFamily::Register => "register",
                    StrategyFamily::StrictIntegrity => "strict_integrity",
                    StrategyFamily::BotControl => "bot_control",
                    StrategyFamily::IdentityFocus => "identity_focus",
                },
                "probe_depth": p.probe_depth,
                "sensitivity": if p.sensitivity_sensitive { "sensitive" } else { "non_sensitive" },
                "shadow_mode": p.shadow_mode,
                "response_profile_default": p.response_profile_default.as_str(),
                "goals": ["fake_browser", "spoof", "antidetect", "rpa", "abnormal", "device_id"],
                "thresholds": {
                    "device_confidence_lt": p.thresholds.device_confidence_lt,
                    "os_score_lt": p.thresholds.os_score_lt,
                    "br_score_lt": p.thresholds.br_score_lt,
                    "rpa_score_lt": p.thresholds.rpa_score_lt,
                    "risk_ge_challenge": p.thresholds.risk_ge_challenge,
                    "risk_ge_deny": p.thresholds.risk_ge_deny,
                    "multi_signal_challenge": p.thresholds.multi_signal_challenge,
                },
            })
        })
        .collect();
    json!({
        "algo": PRODUCT_PUBLIC_ALGO,
        "strategies": items,
        "profiles": ["basic", "standard", "advanced", "diagnostic"],
        "default_strategy_id": "balanced",
        "note": "Admin configures strategy_id + response_profile; mint never changes. Default balanced covers G1–G6.",
        "python_lab_parity": "probe-algo-lab R5 hybrid_r4 redlines; production truth is this Rust module",
    })
}

/// Dictionary dump for console / docs.
pub fn signal_dictionary() -> Value {
    let items: Vec<Value> = SIGNAL_DICT
        .iter()
        .map(|d| {
            json!({
                "code": d.code,
                "category": d.category,
                "severity": d.severity,
                "title_zh": d.title_zh,
                "title_en": d.title_en,
                "description": d.description,
                "decision_hint": d.decision_hint,
            })
        })
        .collect();
    json!({ "algo": PRODUCT_PUBLIC_ALGO, "signals": items })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_human() -> Value {
        json!({
            "session_id": "cycle_clean_human",
            "real_band": "likely_real",
            "product_version": "6.0.22-test",
            "bot": {
                "verdict": "human",
                "score": 0.1,
                "flags": [],
                "family": null
            },
            "product": {
                "device_id": "dv0-aabbccdd11-1122334455-0000000000-0000000000-0000000000-0000000000-0000000000-0000000000-0000000000-0000000000",
                "device_tier": "multi",
                "device_confidence": 0.85,
                "collision_risk": false,
                "association_level": "hardware",
                "digest_path": "real_curves_v1",
                "multi_segment": true,
                "sdk_return": { "emit": true },
                "os": { "score": 0.89, "status": "real", "coverage": 0.8, "reasons": [] },
                "br": { "score": 0.81, "status": "real", "coverage": 0.7, "reasons": [] },
                "rpa": {
                    "score": 0.55, "status": "unknown", "coverage": 0.4,
                    "human_score": 0.55, "bot_score": 0.45,
                    "reasons": ["behavior_early_bound", "rpa_sample_quality_insufficient"]
                },
                "recommended_action": { "action": "allow", "schema": "gr_recommended_action_v1" }
            }
        })
    }

    fn webdriver_bot() -> Value {
        json!({
            "session_id": "cycle_webdriver",
            "real_band": "likely_bot",
            "bot": {
                "verdict": "bot",
                "score": 0.92,
                "flags": ["webdriver", "headless"],
                "family": "selenium"
            },
            "product": {
                "device_id": "dve-76ebdd9bc33c",
                "device_tier": "multi",
                "device_confidence": 0.20,
                "collision_risk": true,
                "association_level": "gateway",
                "sdk_return": { "emit": true },
                "os": { "score": 0.05, "status": "high_risk", "coverage": 0.3, "reasons": ["soft_stack"] },
                "br": { "score": 0.06, "status": "suspect", "coverage": 0.3, "reasons": ["webdriver"] },
                "rpa": {
                    "score": 0.08, "status": "bot", "coverage": 0.5,
                    "bot_score": 0.9, "human_score": 0.1,
                    "reasons": ["webdriver", "matrix_veto.webdriver", "sandbox_capability_dead_rpa_conf"]
                },
                "recommended_action": { "action": "challenge_bot" }
            }
        })
    }

    fn gateway_only() -> Value {
        json!({
            "session_id": "cycle_gateway",
            "real_band": "insufficient",
            "bot": { "verdict": "human", "score": 0.2, "flags": [] },
            "product": {
                "device_id": "dve-323fba82dbd4",
                "device_tier": "multi",
                "device_confidence": 0.21,
                "association_level": "gateway",
                "digest_path": "gateway_only_v1",
                "sdk_return": { "emit": false },
                "os": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "br": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "rpa": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "recommended_action": { "action": "step_up" }
            }
        })
    }

    fn os_spoof_watch() -> Value {
        json!({
            "session_id": "cycle_os_spoof",
            "real_band": "watch",
            "bot": { "verdict": "watch", "score": 0.4, "flags": [] },
            "product": {
                "device_id": "dve-538b0e5c3507",
                "device_confidence": 0.25,
                "sdk_return": { "emit": true },
                "os": { "score": 0.11, "status": "high_risk", "coverage": 0.5, "reasons": ["claim_obs_conflict"] },
                "br": { "score": 0.83, "status": "real", "coverage": 0.6, "reasons": [] },
                "rpa": { "score": 0.34, "status": "bot", "coverage": 0.3, "bot_score": 0.4, "reasons": ["behavior_early_bound"] },
                "recommended_action": { "action": "step_up" }
            }
        })
    }

    #[test]
    fn clean_human_balanced_allow() {
        let out = project_with(&clean_human(), "balanced", "standard", None);
        assert_eq!(out["api_version"], "v6.1");
        assert_eq!(out["bot"]["result"], "not_detected");
        assert_eq!(out["recommended_action"]["action"], "allow");
        assert!(out["identity"]["device_id"].as_str().unwrap().starts_with("dv0-"));
        assert!(out["scores"]["risk"].as_f64().unwrap() < 0.4);
        let codes: Vec<&str> = out["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("code").and_then(|c| c.as_str()))
            .collect();
        assert!(codes.contains(&"bot.not_detected") || codes.contains(&"identity.device_bound"));
        // titles present
        for s in out["signals"].as_array().unwrap() {
            assert!(s.get("title_zh").and_then(|t| t.as_str()).is_some());
        }
        // login_strict alias same family
        let alias = project_with(&clean_human(), "login_strict", "standard", None);
        assert_eq!(alias["recommended_action"]["action"], "allow");
    }

    #[test]
    fn webdriver_denied_balanced() {
        let out = project_with(&webdriver_bot(), "balanced", "standard", None);
        assert_eq!(out["bot"]["result"], "bad");
        assert_eq!(out["recommended_action"]["action"], "deny");
        let codes: Vec<&str> = out["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("code").and_then(|c| c.as_str()))
            .collect();
        assert!(codes.contains(&"bot.webdriver") || codes.contains(&"bot.bad"));
    }

    #[test]
    fn observe_only_never_denies() {
        let out = project_with(&webdriver_bot(), "observe_only", "basic", None);
        assert_eq!(out["recommended_action"]["action"], "allow");
        assert_eq!(
            out["recommended_action"]["would_have_action"].as_str().unwrap(),
            "deny"
        );
    }

    #[test]
    fn gateway_missing_probe_challenges_sensitive() {
        let out = project_with(&gateway_only(), "login_strict", "standard", None);
        assert_eq!(out["identity"]["emit"], false);
        assert!(out["identity"]["device_id"].is_null());
        assert_eq!(out["recommended_action"]["action"], "challenge");
        let codes: Vec<&str> = out["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("code").and_then(|c| c.as_str()))
            .collect();
        assert!(codes.contains(&"identity.missing_probe") || codes.contains(&"identity.withheld"));
    }

    #[test]
    fn os_spoof_challenges() {
        let out = project_with(&os_spoof_watch(), "login_strict", "standard", None);
        let action = out["recommended_action"]["action"].as_str().unwrap();
        assert!(
            action == "challenge" || action == "deny",
            "expected challenge/deny got {action}"
        );
    }

    #[test]
    fn basic_profile_caps_signals() {
        let out = project_with(&webdriver_bot(), "login_strict", "basic", None);
        let n = out["signals"].as_array().unwrap().len();
        assert!(n <= 5, "basic profile should cap signals, got {n}");
        // network/velocity kept compact on basic (G1–G6 inputs); diagnostics stripped
        assert!(out["diagnostics"].is_null());
        assert!(out.get("network").is_some());
    }

    #[test]
    fn checkout_stricter_than_content() {
        let spoof = os_spoof_watch();
        let checkout = project_with(&spoof, "checkout", "advanced", None);
        let content = project_with(&spoof, "content_bot", "standard", None);
        // checkout should not be more lenient than content on spoof
        let rank = |a: &str| match a {
            "allow" => 0,
            "observe" => 1,
            "challenge" => 2,
            "deny" => 3,
            _ => 0,
        };
        let c = rank(checkout["recommended_action"]["action"].as_str().unwrap());
        let t = rank(content["recommended_action"]["action"].as_str().unwrap());
        assert!(c >= t, "checkout should be >= content severity");
    }

    #[test]
    fn mint_device_id_unchanged() {
        let full = clean_human();
        let did = full["product"]["device_id"].as_str().unwrap().to_string();
        let out = project_with(&full, "login_strict", "standard", None);
        assert_eq!(out["identity"]["device_id"].as_str().unwrap(), did);
    }

    #[test]
    fn dictionary_covers_materialized_codes() {
        for case in [clean_human(), webdriver_bot(), gateway_only(), os_spoof_watch()] {
            let out = project_with(&case, "login_strict", "diagnostic", None);
            for s in out["signals"].as_array().unwrap() {
                let code = s["code"].as_str().unwrap();
                assert!(
                    dict_lookup(code).is_some(),
                    "missing dict entry for {code}"
                );
            }
        }
    }

    #[test]
    fn list_presets_includes_goal_ids() {
        let p = list_strategy_presets();
        let ids: Vec<&str> = p["strategies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("strategy_id").and_then(|x| x.as_str()))
            .collect();
        assert!(ids.contains(&"balanced"));
        assert!(ids.contains(&"observe_only"));
        assert!(ids.contains(&"bot_control"));
        assert!(ids.contains(&"strict_integrity"));
        assert_eq!(p["default_strategy_id"], "balanced");
        assert!(p["strategies"].as_array().unwrap().len() >= 6);
    }

    #[test]
    fn rpa_bot_status_alone_not_deny_clean_human() {
        // Real traffic pattern: rpa_status=bot but session human + high os/br
        let mut h = clean_human();
        h["product"]["rpa"] = json!({
            "score": 0.27, "status": "bot", "coverage": 0.4,
            "bot_score": 0.5, "human_score": 0.5,
            "reasons": ["behavior_early_bound", "rpa_sample_quality_insufficient"]
        });
        let out = project_with(&h, "balanced", "standard", None);
        assert_ne!(
            out["recommended_action"]["action"],
            "deny",
            "soft rpa bot must not deny high-trust human"
        );
    }

    fn crawler_traffic() -> Value {
        json!({
            "session_id": "cycle_crawler",
            "real_band": "likely_bot",
            "bot": { "verdict": "crawler", "score": 0.9, "flags": ["crawler"], "family": "bot" },
            "product": {
                "device_id": "dve-crawler01",
                "device_confidence": 0.15,
                "sdk_return": { "emit": false },
                "os": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "br": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "rpa": { "score": 0.0, "status": "missing_probe", "coverage": 0.0, "reasons": [] },
                "recommended_action": { "action": "challenge_bot" }
            }
        })
    }

    fn antidetect_browser() -> Value {
        json!({
            "session_id": "cycle_ad",
            "real_band": "watch",
            "bot": { "verdict": "watch", "score": 0.5, "flags": [] },
            "product": {
                "device_id": "dve-adbrowser",
                "device_confidence": 0.40,
                "sdk_return": { "emit": true },
                "os": { "score": 0.22, "status": "high_risk", "coverage": 0.6, "reasons": ["soft_stack"] },
                "br": { "score": 0.25, "status": "suspect", "coverage": 0.5, "reasons": ["antidetect_vendor"] },
                "rpa": { "score": 0.5, "status": "unknown", "coverage": 0.2, "reasons": [] },
                "antidetect": { "should_downweight": true },
                "recommended_action": { "action": "step_up" }
            }
        })
    }

    /// Python lab redline parity (hybrid_r4 / R5): crawler challenge on balanced, deny on bot_control.
    #[test]
    fn lab_parity_crawler_balanced_challenge_bot_control_deny() {
        let bal = project_with(&crawler_traffic(), "balanced", "standard", None);
        assert_eq!(
            bal["recommended_action"]["action"], "challenge",
            "balanced must challenge crawler (not hard deny like content-only stacks)"
        );
        let botc = project_with(&crawler_traffic(), "bot_control", "standard", None);
        assert_eq!(
            botc["recommended_action"]["action"], "deny",
            "bot_control must deny crawler"
        );
        // content_bot alias
        let alias = project_with(&crawler_traffic(), "content_bot", "standard", None);
        assert_eq!(alias["recommended_action"]["action"], "deny");
    }

    /// G3 antidetect: balanced/strict deny; observe allows with would_have.
    #[test]
    fn lab_parity_antidetect_denied_on_balanced() {
        let out = project_with(&antidetect_browser(), "balanced", "standard", None);
        let codes: Vec<&str> = out["signals"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("code").and_then(|c| c.as_str()))
            .collect();
        assert!(
            codes.contains(&"integrity.anti_detect_browser")
                || codes.contains(&"integrity.br_spoof")
                || codes.contains(&"integrity.os_spoof"),
            "expected integrity signals, got {codes:?}"
        );
        let act = out["recommended_action"]["action"].as_str().unwrap();
        assert!(
            act == "deny" || act == "challenge",
            "AD must not allow under balanced, got {act}"
        );
        let obs = project_with(&antidetect_browser(), "observe_only", "basic", None);
        assert_eq!(obs["recommended_action"]["action"], "allow");
    }

    #[test]
    fn default_strategy_parse_is_balanced() {
        assert_eq!(StrategyId::parse(""), StrategyId::Balanced);
        assert_eq!(StrategyId::parse("hybrid_r4"), StrategyId::Balanced);
        assert_eq!(StrategyId::parse("login_strict").family(), StrategyFamily::Balanced);
        assert_eq!(StrategyId::parse("checkout").family(), StrategyFamily::StrictIntegrity);
        assert_eq!(StrategyId::parse("content_bot").family(), StrategyFamily::BotControl);
    }

    #[test]
    fn missing_probe_never_allow_as_bot_deny() {
        let out = project_with(&gateway_only(), "balanced", "standard", None);
        assert_eq!(out["recommended_action"]["action"], "challenge");
        assert_ne!(out["recommended_action"]["action"], "deny");
    }

    #[test]
    fn result_projection_honors_lane_and_language() {
        let mut fixture = clean_human();
        fixture["product"]["device_id_segments"] = json!({
            "dv0": "dv0-primary",
            "dv4": "dv4-secondary"
        });
        let mut ctx = ProjectCtx::from_query(&{
            let mut q = Map::new();
            q.insert("response_profile".into(), json!("standard"));
            q.insert("primary_device_lane".into(), json!("dv4"));
            q.insert("lang".into(), json!("en"));
            q
        });
        ctx.session_id = Some("cycle_clean_human".into());
        let out = project(&fixture, &ctx);
        assert_eq!(out["identity"]["device_id"], "dv4-secondary");
        assert_eq!(out["identity"]["primary_device_lane"], "dv4");
        assert_eq!(out["meta"]["lang"], "en");
        let bound = out["signals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["code"] == "identity.device_bound")
            .expect("identity signal");
        assert_eq!(bound["title"], "Device identity bound");
    }
}
