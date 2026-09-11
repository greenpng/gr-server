//! Process-wide runtime tuning (fed by admin config publish).
//! Defaults match legacy constants; env still seeds until first publish.
//!
//! **On panel (business knobs)**: cool/TTL/hot-cold/analyze gates/FE retries.
//! **Not on panel**: secrets, DSN, algorithm redlines.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::RwLock;

use crate::{
    CYCLE_COOL_MS, CYCLE_INCOMPLETE_MS, SESSION_HARD_MAX_MS, SESSION_INACTIVITY_MS,
    DEFAULT_COLD_PROMOTE_WINDOW_MS, DEFAULT_COLD_TTL_MS, DEFAULT_HOT_IDLE_MS,
};

#[derive(Debug, Clone)]
pub struct SiteOverride {
    pub cycle_cool_ms: Option<i64>,
    pub cold_ttl_ms: Option<i64>,
    pub analyze_idle_upload_ms: Option<i64>,
    pub return_identity_idle_ms: Option<i64>,
    pub rpa_idle_analyze_ms: Option<i64>,
    pub hard_max_attempts: Option<i64>,
    pub soft_max_attempts: Option<i64>,
    pub collect_enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct RuntimeCfg {
    pub version: u64,
    pub updated_ms: i64,
    pub actor: String,
    // --- cycle / session ---
    pub cycle_cool_ms: i64,
    pub cycle_incomplete_ms: i64,
    pub session_inactivity_ms: i64,
    pub session_hard_max_ms: i64,
    // --- hot / cold ---
    pub hot_idle_ms: i64,
    pub cold_ttl_ms: i64,
    pub cold_promote_window_ms: i64,
    pub cold_purge_interval_ms: u64,
    // --- analyze / return ---
    pub analyze_idle_upload_ms: i64,
    pub analyze_debounce_ms: i64,
    pub return_identity_idle_ms: i64,
    pub rpa_idle_analyze_ms: i64,
    // --- FE retry / budget (non-algorithm) ---
    pub hard_max_attempts: i64,
    pub soft_max_attempts: i64,
    pub deepen_max_attempts: i64,
    pub rpa_max_attempts: i64,
    pub fail_budget_n: i64,
    pub fail_budget_window_ms: i64,
    pub rpa_quiet_ms: i64,
    pub hard_sla_retries: i64,
    pub hard_sla_base_delay_ms: i64,
    pub multi_tick_max: i64,
    pub empty_kick_patience: i64,
    pub upload_max_retries: i64,
    pub client_alive_retry_ms: i64,
    /// FE UploadQueue concurrency (short-visit stream).
    pub upload_concurrency: i64,
    pub upload_mid_ramp: i64,
    pub upload_ramp_after: i64,
    // --- complete policy ---
    /// When true (default), commercial silicon + B10x done → cycle complete/cool.
    pub complete_on_commercial_silicon: bool,
    // --- rate limits (panel hot; 0 = unlimited) ---
    /// Site-wide per-minute caps (0 = unlimited). v1.0.14 policy: aggregate
    /// caps default OFF so a blunt site total can never throttle real users
    /// mixed into bot floods; protection is per-IP on the telemetry route.
    pub rate_limit_open_per_min: i64,
    pub rate_limit_ingest_per_min: i64,
    pub rate_limit_analyze_per_min: i64,
    pub rate_limit_complete_per_min: i64,
    pub rate_limit_result_per_min: i64,
    pub rate_limit_client_event_per_min: i64,
    /// Per-IP per-minute cap for the FE ops telemetry route (default 100 —
    /// a real browser sends a handful of events per session; a bot process
    /// easily exceeds 100/min). 0 = per-IP limiting off.
    pub rate_limit_client_event_per_ip_per_min: i64,
    // --- flood hardening (panel hot) ---
    /// Confirmed-robots (UA-declared crawler) fast lane: no L1 hot, no L3 cold,
    /// no analyze arms — early slim result instead (they never reuse sessions).
    pub robot_fastlane_enabled: bool,
    /// L1 hot map VT cap (0 = unbounded; evict most-idle beyond cap).
    pub hot_max_vts: i64,
    /// Min interval between opportunistic demote/re-arm sweeps on ingest.
    pub arm_sweep_interval_ms: i64,
    /// Max sessions armed per demote sweep (only sessions without any result).
    pub arm_sweep_cap: i64,
    /// Analyze claim batch raised to this while pending > soft cap (drain mode).
    pub analyze_claim_batch_flood: i64,
    pub sites: HashMap<String, SiteOverride>,
}

impl Default for RuntimeCfg {
    fn default() -> Self {
        Self {
            version: 0,
            updated_ms: 0,
            actor: "default".into(),
            cycle_cool_ms: CYCLE_COOL_MS,
            cycle_incomplete_ms: CYCLE_INCOMPLETE_MS,
            session_inactivity_ms: SESSION_INACTIVITY_MS,
            session_hard_max_ms: SESSION_HARD_MAX_MS,
            hot_idle_ms: DEFAULT_HOT_IDLE_MS,
            cold_ttl_ms: DEFAULT_COLD_TTL_MS,
            cold_promote_window_ms: DEFAULT_COLD_PROMOTE_WINDOW_MS,
            cold_purge_interval_ms: 300_000,
            // Short-visit: analyze sooner after last upload
            analyze_idle_upload_ms: 20_000,
            analyze_debounce_ms: 40,
            return_identity_idle_ms: 45_000,
            rpa_idle_analyze_ms: 25_000,
            hard_max_attempts: 8,
            soft_max_attempts: 4,
            deepen_max_attempts: 6,
            rpa_max_attempts: 3,
            fail_budget_n: 24,
            fail_budget_window_ms: 90_000,
            rpa_quiet_ms: 45_000,
            hard_sla_retries: 5,
            hard_sla_base_delay_ms: 2_000,
            multi_tick_max: 96,
            empty_kick_patience: 20,
            upload_max_retries: 5,
            client_alive_retry_ms: 30_000,
            // FE upload stream (panel → open.policy.fe_retry)
            upload_concurrency: 6,
            upload_mid_ramp: 12,
            upload_ramp_after: 14,
            complete_on_commercial_silicon: true,
            // Rate limits (v1.0.14 policy): site totals default 0 (unlimited)
            // — aggregate caps throttled real users mixed into bot floods;
            // per-IP limiting is the protection layer. Env still overrides.
            rate_limit_open_per_min: env_i64("RATE_LIMIT_OPEN_PER_MIN", 0),
            rate_limit_ingest_per_min: env_i64("RATE_LIMIT_INGEST_PER_MIN", 0),
            rate_limit_analyze_per_min: env_i64("RATE_LIMIT_ANALYZE_PER_MIN", 0),
            rate_limit_complete_per_min: env_i64("RATE_LIMIT_COMPLETE_PER_MIN", 0),
            rate_limit_result_per_min: env_i64("RATE_LIMIT_RESULT_PER_MIN", 0),
            rate_limit_client_event_per_min: env_i64("RATE_LIMIT_CLIENT_EVENT_PER_MIN", 0),
            rate_limit_client_event_per_ip_per_min: env_i64(
                "RATE_LIMIT_CLIENT_EVENT_PER_IP_PER_MIN",
                100,
            ),
            robot_fastlane_enabled: true,
            hot_max_vts: env_i64("HOT_MAX_VTS", 8192),
            arm_sweep_interval_ms: 15_000,
            arm_sweep_cap: 256,
            analyze_claim_batch_flood: 16,
            sites: HashMap::new(),
        }
    }
}

fn env_i64(key: &str, default: i64) -> i64 {
    gr_abi::env::get(key)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n >= 0)
        .unwrap_or(default)
}

fn clamp(v: i64, lo: i64, hi: i64) -> i64 {
    v.clamp(lo, hi)
}

/// Clamp business knobs to safe ranges (L0).
pub fn clamp_global(mut c: RuntimeCfg) -> RuntimeCfg {
    c.cycle_cool_ms = clamp(c.cycle_cool_ms, 60_000, 7 * 24 * 3600 * 1000);
    c.cycle_incomplete_ms = clamp(c.cycle_incomplete_ms, 3600_000, 14 * 24 * 3600 * 1000);
    c.session_inactivity_ms = clamp(c.session_inactivity_ms, 60_000, 24 * 3600 * 1000);
    c.session_hard_max_ms = clamp(c.session_hard_max_ms, 3600_000, 7 * 24 * 3600 * 1000);
    c.hot_idle_ms = clamp(c.hot_idle_ms, 60_000, 24 * 3600 * 1000);
    c.cold_ttl_ms = clamp(c.cold_ttl_ms, 3600_000, 90 * 24 * 3600 * 1000);
    c.cold_promote_window_ms = clamp(c.cold_promote_window_ms, 60_000, 7 * 24 * 3600 * 1000);
    c.cold_purge_interval_ms = c.cold_purge_interval_ms.clamp(0, 24 * 3600 * 1000);
    c.analyze_idle_upload_ms = clamp(c.analyze_idle_upload_ms, 5_000, 600_000);
    c.analyze_debounce_ms = clamp(c.analyze_debounce_ms, 10, 5_000);
    c.return_identity_idle_ms = clamp(c.return_identity_idle_ms, 10_000, 600_000);
    c.rpa_idle_analyze_ms = clamp(c.rpa_idle_analyze_ms, 5_000, 600_000);
    c.hard_max_attempts = clamp(c.hard_max_attempts, 1, 32);
    c.soft_max_attempts = clamp(c.soft_max_attempts, 1, 16);
    c.deepen_max_attempts = clamp(c.deepen_max_attempts, 1, 24);
    c.rpa_max_attempts = clamp(c.rpa_max_attempts, 1, 12);
    c.fail_budget_n = clamp(c.fail_budget_n, 3, 200);
    c.fail_budget_window_ms = clamp(c.fail_budget_window_ms, 10_000, 600_000);
    c.rpa_quiet_ms = clamp(c.rpa_quiet_ms, 5_000, 600_000);
    c.hard_sla_retries = clamp(c.hard_sla_retries, 1, 12);
    c.hard_sla_base_delay_ms = clamp(c.hard_sla_base_delay_ms, 1_000, 120_000);
    c.multi_tick_max = clamp(c.multi_tick_max, 8, 256);
    c.empty_kick_patience = clamp(c.empty_kick_patience, 2, 128);
    c.upload_max_retries = clamp(c.upload_max_retries, 1, 16);
    c.client_alive_retry_ms = clamp(c.client_alive_retry_ms, 5_000, 600_000);
    c.upload_concurrency = clamp(c.upload_concurrency, 2, 24);
    c.upload_mid_ramp = clamp(c.upload_mid_ramp, 2, 32);
    c.upload_ramp_after = clamp(c.upload_ramp_after, 4, 32);
    // Rate limits: 0 (off) .. generous ceiling — panel-owned, PG-shared windows.
    c.rate_limit_open_per_min = clamp(c.rate_limit_open_per_min, 0, 1_000_000);
    c.rate_limit_ingest_per_min = clamp(c.rate_limit_ingest_per_min, 0, 1_000_000);
    c.rate_limit_analyze_per_min = clamp(c.rate_limit_analyze_per_min, 0, 1_000_000);
    c.rate_limit_complete_per_min = clamp(c.rate_limit_complete_per_min, 0, 1_000_000);
    c.rate_limit_result_per_min = clamp(c.rate_limit_result_per_min, 0, 1_000_000);
    c.rate_limit_client_event_per_min = clamp(c.rate_limit_client_event_per_min, 0, 1_000_000);
    c.rate_limit_client_event_per_ip_per_min =
        clamp(c.rate_limit_client_event_per_ip_per_min, 0, 1_000_000);
    // Flood hardening knobs.
    c.hot_max_vts = clamp(c.hot_max_vts, 0, 4_000_000);
    c.arm_sweep_interval_ms = clamp(c.arm_sweep_interval_ms, 1_000, 600_000);
    c.arm_sweep_cap = clamp(c.arm_sweep_cap, 1, 10_000);
    c.analyze_claim_batch_flood = clamp(c.analyze_claim_batch_flood, 1, 32);
    c
}

fn cfg_lock() -> &'static RwLock<RuntimeCfg> {
    static L: std::sync::OnceLock<RwLock<RuntimeCfg>> = std::sync::OnceLock::new();
    L.get_or_init(|| RwLock::new(RuntimeCfg::default()))
}

pub fn set_runtime_cfg(c: RuntimeCfg) {
    if let Ok(mut g) = cfg_lock().write() {
        *g = clamp_global(c);
    }
}

pub fn get_runtime_cfg() -> RuntimeCfg {
    cfg_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn config_version() -> u64 {
    get_runtime_cfg().version
}

/// Lab-only: `GR_LAB_DISABLE_COOL=1` or `GR_LAB_FAST_TEST=1` → cycle cool **0ms**.
/// Same-version cool / skip_session_probe will not stick; force re-probe every visit.
/// Never enable on production deploy env.
pub fn lab_disable_cool() -> bool {
    let deploy = gr_abi::env::get("DEPLOY_ENV")
         .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Only honor disable when deploy is lab/dev/test (or unset with explicit flag + lab binary)
    let labish = deploy.is_empty()
        || deploy == "lab"
        || deploy == "dev"
        || deploy == "test"
        || deploy == "local";
    if !labish {
        return false;
    }
    for key in ["GR_LAB_DISABLE_COOL", "GR_LAB_FAST_TEST", "GR_LAB_DISABLE_COOL"] {
        match std::env::var(key)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "1" | "true" | "on" | "yes" => return true,
            _ => {}
        }
    }
    false
}

pub fn cycle_cool_ms() -> i64 {
    if lab_disable_cool() {
        return 0;
    }
    get_runtime_cfg().cycle_cool_ms
}

pub fn cycle_cool_ms_for_site(site_id: &str) -> i64 {
    if lab_disable_cool() {
        return 0;
    }
    if site_id.is_empty() {
        cycle_cool_ms()
    } else {
        effective_for_site(site_id).cycle_cool_ms
    }
}

pub fn cycle_incomplete_ms() -> i64 {
    get_runtime_cfg().cycle_incomplete_ms
}

pub fn cycle_incomplete_ms_for_site(site_id: &str) -> i64 {
    let _ = site_id;
    cycle_incomplete_ms()
}

pub fn session_inactivity_ms_rt() -> i64 {
    get_runtime_cfg().session_inactivity_ms
}

pub fn session_hard_max_ms_rt() -> i64 {
    get_runtime_cfg().session_hard_max_ms
}

pub fn analyze_idle_upload_ms() -> i64 {
    get_runtime_cfg().analyze_idle_upload_ms
}

pub fn return_identity_idle_ms() -> i64 {
    get_runtime_cfg().return_identity_idle_ms
}

pub fn rpa_idle_analyze_ms() -> i64 {
    get_runtime_cfg().rpa_idle_analyze_ms
}

pub fn cold_purge_interval_ms() -> u64 {
    get_runtime_cfg().cold_purge_interval_ms
}

/// Flood hardening: ingest-opportunistic demote/re-arm sweep minimum interval.
pub fn arm_sweep_interval_ms() -> i64 {
    get_runtime_cfg().arm_sweep_interval_ms
}

/// Flood hardening: max sessions armed per demote sweep (result-less only).
pub fn arm_sweep_cap() -> i64 {
    get_runtime_cfg().arm_sweep_cap
}

/// Robots fast lane (UA-declared crawler sessions skip L1/L3/arms).
pub fn robot_fastlane_enabled() -> bool {
    get_runtime_cfg().robot_fastlane_enabled
}

/// Flood drain mode: claim batch raised while pending > soft cap.
pub fn analyze_claim_batch_flood() -> usize {
    get_runtime_cfg().analyze_claim_batch_flood.max(1) as usize
}

pub fn complete_on_commercial_silicon() -> bool {
    get_runtime_cfg().complete_on_commercial_silicon
}

/// FE probe_lifecycle / upload policy snapshot (for open/bootstrap).
pub fn fe_retry_policy_json(c: &RuntimeCfg) -> Value {
    // Geometric SLA delays from base: base, 2x, ~3.6x, 6x (capped)
    let base = c.hard_sla_base_delay_ms;
    let mut sla = Vec::new();
    let mut d = base;
    for _ in 0..c.hard_sla_retries.max(1) {
        sla.push(d);
        d = (d as f64 * 1.8) as i64;
        if d > 60_000 {
            d = 60_000;
        }
    }
    json!({
        "hard_max_attempts": c.hard_max_attempts,
        "soft_max_attempts": c.soft_max_attempts,
        "deepen_max_attempts": c.deepen_max_attempts,
        "rpa_max_attempts": c.rpa_max_attempts,
        "fail_budget_n": c.fail_budget_n,
        "fail_budget_window_ms": c.fail_budget_window_ms,
        "rpa_quiet_ms": c.rpa_quiet_ms,
        "hard_sla_retries": c.hard_sla_retries,
        "hard_sla_base_delay_ms": c.hard_sla_base_delay_ms,
        "hard_sla_delays_ms": sla,
        "multi_tick_max": c.multi_tick_max,
        "empty_kick_patience": c.empty_kick_patience,
        "upload_max_retries": c.upload_max_retries,
        "client_alive_retry_ms": c.client_alive_retry_ms,
        "upload_concurrency": c.upload_concurrency,
        "upload_mid_ramp": c.upload_mid_ramp,
        "upload_ramp_after": c.upload_ramp_after,
        "complete_on_commercial_silicon": c.complete_on_commercial_silicon,
    })
}

pub fn effective_for_site(site_id: &str) -> RuntimeCfg {
    let mut c = get_runtime_cfg();
    if site_id.is_empty() {
        return c;
    }
    if let Some(o) = c.sites.get(site_id).cloned() {
        if let Some(v) = o.cycle_cool_ms {
            c.cycle_cool_ms = v;
        }
        if let Some(v) = o.cold_ttl_ms {
            c.cold_ttl_ms = v;
        }
        if let Some(v) = o.analyze_idle_upload_ms {
            c.analyze_idle_upload_ms = v;
        }
        if let Some(v) = o.return_identity_idle_ms {
            c.return_identity_idle_ms = v;
        }
        if let Some(v) = o.rpa_idle_analyze_ms {
            c.rpa_idle_analyze_ms = v;
        }
        if let Some(v) = o.hard_max_attempts {
            c.hard_max_attempts = v;
        }
        if let Some(v) = o.soft_max_attempts {
            c.soft_max_attempts = v;
        }
        c = clamp_global(c);
    }
    c
}

pub fn global_to_json(c: &RuntimeCfg) -> Value {
    let mut m = json!({
        "version": c.version,
        "updated_ms": c.updated_ms,
        "actor": c.actor,
        "cycle_cool_ms": c.cycle_cool_ms,
        "cycle_incomplete_ms": c.cycle_incomplete_ms,
        "session_inactivity_ms": c.session_inactivity_ms,
        "session_hard_max_ms": c.session_hard_max_ms,
        "hot_idle_ms": c.hot_idle_ms,
        "cold_ttl_ms": c.cold_ttl_ms,
        "cold_promote_window_ms": c.cold_promote_window_ms,
        "cold_purge_interval_ms": c.cold_purge_interval_ms,
        "analyze_idle_upload_ms": c.analyze_idle_upload_ms,
        "analyze_debounce_ms": c.analyze_debounce_ms,
        "return_identity_idle_ms": c.return_identity_idle_ms,
        "rpa_idle_analyze_ms": c.rpa_idle_analyze_ms,
        "hard_max_attempts": c.hard_max_attempts,
        "soft_max_attempts": c.soft_max_attempts,
        "deepen_max_attempts": c.deepen_max_attempts,
        "rpa_max_attempts": c.rpa_max_attempts,
        "fail_budget_n": c.fail_budget_n,
        "fail_budget_window_ms": c.fail_budget_window_ms,
        "rpa_quiet_ms": c.rpa_quiet_ms,
        "hard_sla_retries": c.hard_sla_retries,
        "hard_sla_base_delay_ms": c.hard_sla_base_delay_ms,
        "multi_tick_max": c.multi_tick_max,
        "empty_kick_patience": c.empty_kick_patience,
        "upload_max_retries": c.upload_max_retries,
        "client_alive_retry_ms": c.client_alive_retry_ms,
        "upload_concurrency": c.upload_concurrency,
        "upload_mid_ramp": c.upload_mid_ramp,
        "upload_ramp_after": c.upload_ramp_after,
        "complete_on_commercial_silicon": c.complete_on_commercial_silicon,
        "rate_limit_open_per_min": c.rate_limit_open_per_min,
        "rate_limit_ingest_per_min": c.rate_limit_ingest_per_min,
        "rate_limit_analyze_per_min": c.rate_limit_analyze_per_min,
        "rate_limit_complete_per_min": c.rate_limit_complete_per_min,
        "rate_limit_result_per_min": c.rate_limit_result_per_min,
        "rate_limit_client_event_per_min": c.rate_limit_client_event_per_min,
        "rate_limit_client_event_per_ip_per_min": c.rate_limit_client_event_per_ip_per_min,
        "robot_fastlane_enabled": c.robot_fastlane_enabled,
        "hot_max_vts": c.hot_max_vts,
        "arm_sweep_interval_ms": c.arm_sweep_interval_ms,
        "arm_sweep_cap": c.arm_sweep_cap,
        "analyze_claim_batch_flood": c.analyze_claim_batch_flood,
        "fe_retry_policy": fe_retry_policy_json(c),
        "field_help": field_help_json(),
        "clamp_notes": {
            "return_identity_idle_ms_min": 10_000,
            "cycle_cool_ms_min": 60_000,
            "hard_max_attempts_max": 32,
        }
    });
    // ensure object
    let _ = m.as_object_mut();
    m
}

/// UI/docs: human labels for each knob (Chinese).
pub fn field_help_json() -> Value {
    json!({
        "cycle_cool_ms": "身份周期冷却：complete 后同 VT 不再重探（默认 24h）",
        "cycle_incomplete_ms": "未完成周期最长保留（默认 72h）",
        "session_inactivity_ms": "会话无活动窗口（遗留；主时钟以 cool/incomplete 为准）",
        "session_hard_max_ms": "会话硬上限（遗留）",
        "hot_idle_ms": "L1 热缓存空闲降级时间",
        "cold_ttl_ms": "L3 冷数据 TTL，超时 purge",
        "cold_promote_window_ms": "冷→热提升窗口",
        "cold_purge_interval_ms": "定时 purge 间隔（0=仅 env/机会式）",
        "analyze_idle_upload_ms": "无新上传多久触发分析",
        "analyze_debounce_ms": "分析任务 debounce",
        "return_identity_idle_ms": "SDK 返回身份最短空闲（≥10s）",
        "rpa_idle_analyze_ms": "RPA 安静后分析窗",
        "hard_max_attempts": "FE 硬包（B10 等）最大重试次数",
        "soft_max_attempts": "FE soft/mid 包最大重试",
        "deepen_max_attempts": "B10x 加深包最大重试",
        "rpa_max_attempts": "B11 RPA 包最大重试",
        "fail_budget_n": "滚动窗口内失败次数上限→quiet",
        "fail_budget_window_ms": "失败预算窗口",
        "rpa_quiet_ms": "RPA 耗尽后安静时长",
        "hard_sla_retries": "B10 缺失时 SLA 再踢次数",
        "hard_sla_base_delay_ms": "SLA 再踢基础延迟（几何递增）",
        "multi_tick_max": "analyze↔route 最大循环（安全帽）",
        "empty_kick_patience": "连续空 kick 容忍 tick 数",
        "upload_max_retries": "上传队列最大重试",
        "client_alive_retry_ms": "客户端保活/重试间隔",
        "upload_concurrency": "上传并发起始（短访问边采边传）",
        "upload_mid_ramp": "B0 入队后 mid 并发",
        "upload_ramp_after": "首包成功后目标并发",
        "analyze_idle_upload_ms": "无新上传多久触发分析（短访问默认 20s）",
        "complete_on_commercial_silicon": "商业硅材料+B10x 齐→关周期冷却（默认开；178 修复）",
        "rate_limit_open_per_min": "open 每分钟每站总上限（0=不限，默认 0；共享 PG 窗口）",
        "rate_limit_ingest_per_min": "ingest 每分钟每站总上限（0=不限，默认 0）",
        "rate_limit_analyze_per_min": "analyze 每分钟每站总上限（0=不限，默认 0）",
        "rate_limit_complete_per_min": "complete 每分钟每站总上限（0=不限，默认 0）",
        "rate_limit_result_per_min": "result 每分钟每站总上限（0=不限，默认 0）",
        "rate_limit_client_event_per_min": "client_event 每分钟每站总上限（0=不限，默认 0）",
        "rate_limit_client_event_per_ip_per_min": "client_event 每分钟每 IP 上限（默认 100；0=不限。单 IP 超限只掐该 IP，不影响其他访客）",
        "robot_fastlane_enabled": "已确认爬虫快道：不入 L1/不冷存/不挂分析臂，直接早判结果（默认开）",
        "hot_max_vts": "L1 热图 VT 封顶（0=不封；超出按最久未活跃逐出）",
        "arm_sweep_interval_ms": "ingest 顺带降级/挂臂扫最小间隔（防重臂风暴）",
        "arm_sweep_cap": "每次扫臂最多新挂会话数（仅无结果会话）",
        "analyze_claim_batch_flood": "积压超软上限时的领取批量（排水模式）"
    })
}

pub fn sites_to_json(c: &RuntimeCfg) -> Value {
    let mut m = Map::new();
    for (k, o) in &c.sites {
        m.insert(
            k.clone(),
            json!({
                "cycle_cool_ms": o.cycle_cool_ms,
                "cold_ttl_ms": o.cold_ttl_ms,
                "analyze_idle_upload_ms": o.analyze_idle_upload_ms,
                "return_identity_idle_ms": o.return_identity_idle_ms,
                "rpa_idle_analyze_ms": o.rpa_idle_analyze_ms,
                "hard_max_attempts": o.hard_max_attempts,
                "soft_max_attempts": o.soft_max_attempts,
                "collect_enabled": o.collect_enabled,
            }),
        );
    }
    Value::Object(m)
}

fn patch_i64(c: &mut i64, patch: &Value, k: &str) {
    if let Some(v) = patch.get(k).and_then(|x| x.as_i64()) {
        *c = v;
    }
}

fn patch_u64(c: &mut u64, patch: &Value, k: &str) {
    if let Some(v) = patch.get(k).and_then(|x| x.as_u64()) {
        *c = v;
    }
}

pub fn parse_global_patch(base: &RuntimeCfg, patch: &Value) -> RuntimeCfg {
    let mut c = base.clone();
    patch_i64(&mut c.cycle_cool_ms, patch, "cycle_cool_ms");
    patch_i64(&mut c.cycle_incomplete_ms, patch, "cycle_incomplete_ms");
    patch_i64(&mut c.session_inactivity_ms, patch, "session_inactivity_ms");
    patch_i64(&mut c.session_hard_max_ms, patch, "session_hard_max_ms");
    patch_i64(&mut c.hot_idle_ms, patch, "hot_idle_ms");
    patch_i64(&mut c.cold_ttl_ms, patch, "cold_ttl_ms");
    patch_i64(&mut c.cold_promote_window_ms, patch, "cold_promote_window_ms");
    patch_u64(&mut c.cold_purge_interval_ms, patch, "cold_purge_interval_ms");
    patch_i64(&mut c.analyze_idle_upload_ms, patch, "analyze_idle_upload_ms");
    patch_i64(&mut c.analyze_debounce_ms, patch, "analyze_debounce_ms");
    patch_i64(&mut c.return_identity_idle_ms, patch, "return_identity_idle_ms");
    patch_i64(&mut c.rpa_idle_analyze_ms, patch, "rpa_idle_analyze_ms");
    patch_i64(&mut c.hard_max_attempts, patch, "hard_max_attempts");
    patch_i64(&mut c.soft_max_attempts, patch, "soft_max_attempts");
    patch_i64(&mut c.deepen_max_attempts, patch, "deepen_max_attempts");
    patch_i64(&mut c.rpa_max_attempts, patch, "rpa_max_attempts");
    patch_i64(&mut c.fail_budget_n, patch, "fail_budget_n");
    patch_i64(&mut c.fail_budget_window_ms, patch, "fail_budget_window_ms");
    patch_i64(&mut c.rpa_quiet_ms, patch, "rpa_quiet_ms");
    patch_i64(&mut c.hard_sla_retries, patch, "hard_sla_retries");
    patch_i64(&mut c.hard_sla_base_delay_ms, patch, "hard_sla_base_delay_ms");
    patch_i64(&mut c.multi_tick_max, patch, "multi_tick_max");
    patch_i64(&mut c.empty_kick_patience, patch, "empty_kick_patience");
    patch_i64(&mut c.upload_max_retries, patch, "upload_max_retries");
    patch_i64(&mut c.client_alive_retry_ms, patch, "client_alive_retry_ms");
    patch_i64(&mut c.upload_concurrency, patch, "upload_concurrency");
    patch_i64(&mut c.upload_mid_ramp, patch, "upload_mid_ramp");
    patch_i64(&mut c.upload_ramp_after, patch, "upload_ramp_after");
    patch_i64(&mut c.rate_limit_open_per_min, patch, "rate_limit_open_per_min");
    patch_i64(&mut c.rate_limit_ingest_per_min, patch, "rate_limit_ingest_per_min");
    patch_i64(&mut c.rate_limit_analyze_per_min, patch, "rate_limit_analyze_per_min");
    patch_i64(&mut c.rate_limit_complete_per_min, patch, "rate_limit_complete_per_min");
    patch_i64(&mut c.rate_limit_result_per_min, patch, "rate_limit_result_per_min");
    patch_i64(
        &mut c.rate_limit_client_event_per_min,
        patch,
        "rate_limit_client_event_per_min",
    );
    patch_i64(
        &mut c.rate_limit_client_event_per_ip_per_min,
        patch,
        "rate_limit_client_event_per_ip_per_min",
    );
    patch_i64(&mut c.hot_max_vts, patch, "hot_max_vts");
    patch_i64(&mut c.arm_sweep_interval_ms, patch, "arm_sweep_interval_ms");
    patch_i64(&mut c.arm_sweep_cap, patch, "arm_sweep_cap");
    patch_i64(
        &mut c.analyze_claim_batch_flood,
        patch,
        "analyze_claim_batch_flood",
    );
    if let Some(v) = patch.get("complete_on_commercial_silicon").and_then(|x| x.as_bool()) {
        c.complete_on_commercial_silicon = v;
    }
    if let Some(v) = patch.get("robot_fastlane_enabled").and_then(|x| x.as_bool()) {
        c.robot_fastlane_enabled = v;
    }
    clamp_global(c)
}

pub fn parse_site_override(v: &Value) -> SiteOverride {
    SiteOverride {
        cycle_cool_ms: v.get("cycle_cool_ms").and_then(|x| x.as_i64()),
        cold_ttl_ms: v.get("cold_ttl_ms").and_then(|x| x.as_i64()),
        analyze_idle_upload_ms: v.get("analyze_idle_upload_ms").and_then(|x| x.as_i64()),
        return_identity_idle_ms: v.get("return_identity_idle_ms").and_then(|x| x.as_i64()),
        rpa_idle_analyze_ms: v.get("rpa_idle_analyze_ms").and_then(|x| x.as_i64()),
        hard_max_attempts: v.get("hard_max_attempts").and_then(|x| x.as_i64()),
        soft_max_attempts: v.get("soft_max_attempts").and_then(|x| x.as_i64()),
        collect_enabled: v.get("collect_enabled").and_then(|x| x.as_bool()),
    }
}

pub fn cfg_from_stored(
    global: &Value,
    sites: &Value,
    version: u64,
    actor: &str,
    updated_ms: i64,
) -> RuntimeCfg {
    let mut c = RuntimeCfg::default();
    c = parse_global_patch(&c, global);
    c.version = version;
    c.actor = actor.into();
    c.updated_ms = updated_ms;
    if let Some(obj) = sites.as_object() {
        for (k, v) in obj {
            c.sites.insert(k.clone(), parse_site_override(v));
        }
    }
    clamp_global(c)
}
