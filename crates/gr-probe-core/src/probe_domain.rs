//! Probe domain deepen policy — taxonomy + scheduling contracts.
//!
//! **Redline:** deepen only for this-session measurement completeness / capability
//! adaptation. Never target peer device_id or mean clusters.
//!
//! Taxonomy (do not collapse into one "abnormal kernel" label):
//! 1. **stack_anomaly** — claim≠obs, form/platform migration-like conflicts
//! 2. **channel_unconventional** — legitimate but non-ANGLE measurement basis
//!    (WebKitGTK, soft GL, unknown engine)
//! 3. **capability_thin** — old/missing APIs; prefer legacy pack or gap report
//! 4. **contract_incomplete** — normal stack, materials not filled yet
//!
//! B10x is the residual-domain specialized deepen pack family only.
//! Other domains use existing deep packs (B46_audio_deep, B52_…), not B*x clones.
//!
//! See `v5-docs/architecture/probe-domain-deepen.md`.

use serde_json::{json, Map, Value};

/// Why this session may need deepen (ops / brain notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepenClass {
    /// Claim/obs or form/platform conflict — authenticity packs first.
    StackAnomaly,
    /// Different residual/HW measurement channel (WebKit, soft GL, unknown).
    ChannelUnconventional,
    /// Missing APIs / too old — legacy path or gap, do not thrash.
    CapabilityThin,
    /// Normal stack; contract bits missing.
    ContractIncomplete,
    /// Directions yield mid-band (handled by UCB; listed for completeness).
    YieldPartial,
    /// No deepen.
    None,
}

impl DeepenClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DeepenClass::StackAnomaly => "stack_anomaly",
            DeepenClass::ChannelUnconventional => "channel_unconventional",
            DeepenClass::CapabilityThin => "capability_thin",
            DeepenClass::ContractIncomplete => "contract_incomplete",
            DeepenClass::YieldPartial => "yield_partial",
            DeepenClass::None => "none",
        }
    }
}

/// One product/measurement domain for bounded deepen.
#[derive(Debug, Clone)]
pub struct ProbeDomain {
    pub id: &'static str,
    pub primary_packs: &'static [&'static str],
    pub deepen_packs: &'static [&'static str],
    /// Product axes this domain mainly serves.
    pub axes: &'static [&'static str],
    /// HW serial class: "hw" | "light" | "verify"
    pub serial_class: &'static str,
    /// Default max deepen packs landed this session (band may raise).
    pub max_deepen_ticks: usize,
    pub note: &'static str,
}

/// SSOT domain table — residual is fully wired; others are policy map for brain/ops.
pub const PROBE_DOMAINS: &[ProbeDomain] = &[
    ProbeDomain {
        id: "device_residual",
        primary_packs: &["B10_hw_curves", "B2_hardware"],
        deepen_packs: &[
            "B10x_silicon_ulp",
            "B10x_silicon_rint",
            "B10x_silicon_noderiv",
            "B10x_webkit_gl_noise",
            "B10x_webkit_wave2_warm4",
            "B10x_softgl_hedge",
            "B10x_legacy_webgl1",
            "B10x_unknown_kernel",
            "B10x_angle_crosscheck",
        ],
        axes: &["device_id", "os", "br"],
        serial_class: "hw",
        max_deepen_ticks: 1,
        note: "Only domain with B*x specialized multipath; union residual_paths",
    },
    ProbeDomain {
        id: "gpu_physical",
        primary_packs: &["B2_hardware", "B17_hw_physical"],
        deepen_packs: &[
            "B22_gpu_timer",
            "B30_gpu_bandwidth",
            "B31_shader_numeric",
            "B33_caps_pressure",
            "B18_webgpu",
            "B59_webgl_params_full",
            "B60_webgl_extensions_full",
        ],
        axes: &["device_id", "os", "br"],
        serial_class: "hw",
        max_deepen_ticks: 2,
        note: "Use existing hard packs as deepen — no B17x",
    },
    ProbeDomain {
        id: "audio_media",
        primary_packs: &["B10_hw_curves", "B46_audio_deep"],
        deepen_packs: &["B61_audio_worklet", "B62_offline_audio_moments", "B19_eme_media"],
        axes: &["device_id", "os"],
        serial_class: "hw",
        max_deepen_ticks: 1,
        note: "Audio deep packs act as deepen role",
    },
    ProbeDomain {
        id: "browser_kernel",
        primary_packs: &["B0_bootstrap", "B1_conflict", "B12_anti_camouflage"],
        deepen_packs: &[
            "B26_agent_parity",
            "B38_neg_dict",
            "B43_errors_engine",
            "B23_native_canvas_hedge",
            "B47_api_flags_detail",
            "B52_navigator_deep",
            "B53_window_keys_deep",
            "B65_client_hints_full",
        ],
        axes: &["br"],
        serial_class: "light",
        max_deepen_ticks: 2,
        note: "stack_anomaly primary home — not residual B10x",
    },
    ProbeDomain {
        id: "sandbox_xsrc",
        primary_packs: &["B7_sandbox"],
        deepen_packs: &[
            "B15_cross_curves",
            "B77_worker_env_deep",
            "B78_iframe_env_deep",
            "B79_cross_origin_isolation",
        ],
        axes: &["br", "device_id"],
        serial_class: "hw",
        max_deepen_ticks: 1,
        note: "Multi-source consistency; B7 is serial HW-class",
    },
    ProbeDomain {
        id: "os_env",
        primary_packs: &["B3_system", "B2_hardware"],
        deepen_packs: &[
            "B4_mobile",
            "B16_fast_signals",
            "B29_sensors_battery",
            "B45_display_hdr",
            "B64_timezone_deep",
        ],
        axes: &["os", "device_id"],
        serial_class: "light",
        max_deepen_ticks: 1,
        note: "form/platform/env density",
    },
    ProbeDomain {
        id: "net_edge",
        primary_packs: &["B8_gateway_early", "B9_network"],
        deepen_packs: &[
            "B55_webrtc_ice_deep",
            "B56_webrtc_stats",
            "B40_websocket_fp",
            "B66_sec_ch_headers",
        ],
        axes: &["os", "br"],
        serial_class: "light",
        max_deepen_ticks: 1,
        note: "B8 is L0 anchor not deepen",
    },
    ProbeDomain {
        id: "rpa_bio",
        primary_packs: &["B11_interaction"],
        deepen_packs: &["B11_interaction"],
        axes: &["rpa"],
        serial_class: "light",
        max_deepen_ticks: 1,
        note: "Same pack continues event collection; not B11x",
    },
    ProbeDomain {
        id: "census_surface",
        primary_packs: &["B5_census", "B21_census_volume"],
        deepen_packs: &[
            "B47_api_flags_detail",
            "B50_font_matrix_detail",
            "B51_mq_matrix_detail",
            "B48_css_supports_detail",
            "B63_intl_full",
        ],
        axes: &["br", "os"],
        serial_class: "light",
        max_deepen_ticks: 1,
        note: "Dense optional; low band should not prior-deepen",
    },
    ProbeDomain {
        id: "challenge_pohw",
        primary_packs: &["B20_challenge_seed"],
        deepen_packs: &["B22_gpu_timer", "B76_math_wasm_deep"],
        axes: &["device_id", "br"],
        serial_class: "hw",
        max_deepen_ticks: 1,
        note: "Band-gated; high/critical only typically",
    },
    ProbeDomain {
        id: "automation_control",
        primary_packs: &["B12_anti_camouflage", "B1_conflict"],
        deepen_packs: &["B26_agent_parity", "B6_risk", "B38_neg_dict"],
        axes: &["rpa", "br"],
        serial_class: "light",
        max_deepen_ticks: 1,
        note: "CDP/webdriver surface; stack_anomaly companion",
    },
];

pub fn domain_by_id(id: &str) -> Option<&'static ProbeDomain> {
    PROBE_DOMAINS.iter().find(|d| d.id == id)
}

fn str_field(fo: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = fo.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn bool_field(fo: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| {
        fo.get(*k)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    })
}

/// Classify this session's stack for deepen routing (not identity merge).
///
/// Judgment is **sample-library first** (`stack_cooccurrence_prior_v1`), then
/// local capability/GL flags. Epiphany/WebKitGTK on Linux → rare_native + B10x
/// channel prior (not Safari migration). Safari brand on Linux → migration_suspect.
pub fn classify_probe_stack_profile(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();

    let eng = str_field(
        &fo,
        &[
            "residual_probe_engine",
            "engine_family",
            "engine_obs",
            "engine_claim",
        ],
    )
    .unwrap_or_else(|| "unknown".into())
    .to_ascii_lowercase();

    let claim = str_field(&fo, &["engine_claim"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let obs = str_field(&fo, &["engine_obs"])
        .unwrap_or_default()
        .to_ascii_lowercase();

    let gl = crate::engine_surface::classify_gl_stack(&fo).to_ascii_lowercase();

    // --- Sample library (population co-occurrence) ---
    let co = crate::stack_cooccurrence::lookup_stack_cooccurrence(fields);
    let co_status = co
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("uncommon_native");
    let co_anomaly = co
        .get("stack_anomaly")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let co_channel = co
        .get("channel_unconventional")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let co_b10x = co
        .get("b10x_prior")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let co_skip = co
        .get("residual_skip_silicon")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let co_domains: Vec<String> = co
        .get("preferred_domains")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Local anomaly flags (supplement library conflict_rules)
    let mut anomaly_flags: Vec<String> = Vec::new();
    if let Some(hits) = co.get("conflict_hits").and_then(|v| v.as_array()) {
        for h in hits {
            if let Some(s) = h.as_str() {
                anomaly_flags.push(s.to_string());
            }
        }
    }
    if co_status == "migration_suspect" {
        anomaly_flags.push("sample_migration_suspect".into());
    }
    if co_status == "impossible" {
        anomaly_flags.push("sample_impossible".into());
    }
    if bool_field(&fo, &["webdriver"]) || bool_field(&fo, &["automation"]) {
        anomaly_flags.push("automation_surface".into());
    }
    let stack_anomaly = co_anomaly
        || anomaly_flags.iter().any(|f| {
            f != "automation_surface" && f != "soft_gl_desktop"
        });

    // Channel flags: library + GL surface
    let mut channel_flags: Vec<String> = Vec::new();
    if co_channel {
        channel_flags.push("sample_channel".into());
    }
    if eng == "webkit" || gl.contains("webkit") {
        channel_flags.push("webkit_channel".into());
    }
    if eng == "unknown" {
        channel_flags.push("unknown_engine".into());
    }
    if gl.contains("software")
        || gl.contains("swiftshader")
        || gl.contains("llvmpipe")
        || fo.get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        channel_flags.push("soft_gl".into());
    }
    let unmask_blocked = fo
        .get("webgl_unmask_blocked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || {
            let v = str_field(&fo, &["webgl_unmasked_vendor", "webgl_vendor"])
                .unwrap_or_default()
                .to_ascii_lowercase();
            let r = str_field(&fo, &["webgl_unmasked_renderer", "webgl_renderer"])
                .unwrap_or_default();
            v.contains("apple") && r.is_empty()
        };
    if unmask_blocked {
        channel_flags.push("webgl_unmask_blocked".into());
    }
    let channel_unconventional = !channel_flags.is_empty() || co_channel;

    // capability_thin
    let mut thin_flags: Vec<&str> = Vec::new();
    if fo.get("webgl2").and_then(|v| v.as_bool()) == Some(false)
        || fo.get("webgl2_supported").and_then(|v| v.as_bool()) == Some(false)
    {
        thin_flags.push("no_webgl2");
    }
    if fo.get("offline_audio_available").and_then(|v| v.as_bool()) == Some(false) {
        thin_flags.push("no_offline_audio");
    }
    if fo.get("webrtc_available").and_then(|v| v.as_bool()) == Some(false)
        || fo.get("rtc_peer_connection").and_then(|v| v.as_bool()) == Some(false)
    {
        thin_flags.push("no_webrtc");
    }
    if let Some(ver) = str_field(&fo, &["browser_version", "br_version", "ua_full_version"]) {
        if let Some(maj) = ver.split('.').next().and_then(|s| s.parse::<u32>().ok()) {
            let name = str_field(&fo, &["browser_name", "br_name"])
                .unwrap_or_default()
                .to_ascii_lowercase();
            if (name.contains("chrome") || name.contains("chromium") || name.contains("edge"))
                && maj > 0
                && maj < 80
            {
                thin_flags.push("browser_major_old");
            }
            if name.contains("firefox") && maj > 0 && maj < 70 {
                thin_flags.push("browser_major_old");
            }
        }
    }
    let capability_thin = !thin_flags.is_empty();

    // B10x prior: sample library is authority (Epiphany rare_native, Safari Linux suspect, iOS WebKit…)
    let residual_skip_silicon = co_skip
        || (channel_flags.iter().any(|f| f == "soft_gl")
            && !channel_flags.iter().any(|f| f == "webkit_channel")
            && !channel_flags.iter().any(|f| f == "unknown_engine"));
    let residual_channel_prior = (co_b10x || channel_unconventional) && !residual_skip_silicon;

    // preferred domains: sample library first, then class defaults
    let mut preferred_domains: Vec<String> = co_domains;
    if stack_anomaly {
        for d in ["browser_kernel", "automation_control", "sandbox_xsrc"] {
            if !preferred_domains.iter().any(|x| x == d) {
                preferred_domains.push(d.into());
            }
        }
    }
    if residual_channel_prior {
        for d in ["device_residual", "gpu_physical"] {
            if !preferred_domains.iter().any(|x| x == d) {
                preferred_domains.push(d.into());
            }
        }
    }
    if capability_thin {
        for d in ["device_residual", "browser_kernel"] {
            if !preferred_domains.iter().any(|x| x == d) {
                preferred_domains.push(d.into());
            }
        }
    }
    if preferred_domains.is_empty() {
        preferred_domains.push("device_residual".into());
    }

    let primary_class = if stack_anomaly {
        DeepenClass::StackAnomaly
    } else if channel_unconventional || residual_channel_prior {
        DeepenClass::ChannelUnconventional
    } else if capability_thin {
        DeepenClass::CapabilityThin
    } else {
        DeepenClass::None
    };

    json!({
        "algo": "probe_stack_profile_v2",
        "policy": "measurement_not_identity",
        "force_merge": false,
        "engine_family": eng,
        "engine_claim": if claim.is_empty() { Value::Null } else { json!(claim) },
        "engine_obs": if obs.is_empty() { Value::Null } else { json!(obs) },
        "gl_stack_class": gl,
        "sample": {
            "status": co_status,
            "rate": co.get("rate").cloned().unwrap_or(Value::Null),
            "rarity": co.get("rarity").cloned().unwrap_or(Value::Null),
            "matched_tuple": co.get("matched_tuple").cloned().unwrap_or(Value::Null),
            "lookup_key": co.get("lookup_key").cloned().unwrap_or(Value::Null),
            "dims": co.get("dims").cloned().unwrap_or(Value::Null),
            "tuple_note": co.get("tuple_note").cloned().unwrap_or(Value::Null),
            "anomaly_score": co.get("anomaly_score").cloned().unwrap_or(Value::Null),
        },
        "stack_anomaly": stack_anomaly,
        "channel_unconventional": channel_unconventional,
        "capability_thin": capability_thin,
        "anomaly_flags": anomaly_flags,
        "channel_flags": channel_flags,
        "thin_flags": thin_flags,
        "primary_class": primary_class.as_str(),
        "residual_channel_prior": residual_channel_prior,
        "residual_skip_silicon": residual_skip_silicon,
        "preferred_domains": preferred_domains,
        "note": "Sample library drives rare_native (Epiphany/Linux→B10x) vs migration_suspect (Safari/Linux); never peer-dh merge",
    })
}

/// Residual-domain B10x schedule — **always open after B10**.
///
/// Product decision (2026-08-02): do **not** gate B10x on measurement_complete,
/// soft_stack, OS/br, or sample library. Run residual deepen packs so multipath
/// variants land for cross-browser comparison. Only require B10 first and
/// `b10x_ticks < max` (cap = full B10x catalog size).
///
/// Stack profile is ops-only (`schedule_gate: false`).
pub fn residual_deepen_decision(
    fields: &Value,
    measurement_complete: bool,
    b10_present: bool,
    b10x_ticks: usize,
    max_ticks: usize,
    soft_stack: bool,
) -> Value {
    let _ = soft_stack; // no longer a schedule gate
    let profile = classify_probe_stack_profile(fields);

    // Always open residual deepen after B10 until catalog ticks exhausted.
    let (reason, worth) = if !b10_present {
        (None, false)
    } else if b10x_ticks >= max_ticks {
        (None, false)
    } else {
        // Prefer explicit reason for ops; never stop early for "complete".
        let reason = if measurement_complete {
            "always_open_b10x_after_b10"
        } else {
            "always_open_b10x_contract_also_incomplete"
        };
        (Some(reason), true)
    };

    json!({
        "worth_deepen": worth,
        "deepen_reason": reason.map(|s| json!(s)).unwrap_or(Value::Null),
        "stack_profile": profile,
        "stack_profile_schedule_gate": false,
        "domain": "device_residual",
        "policy": "always_open_b10x_after_b10",
        "never_peer_dh": true,
        "note": "B10x always scheduled after B10 (no measurement_complete/OS/br gate); multi-path data for cross-browser true-curve research",
    })
}

/// Summarize all domains for ops (present packs → progress hint).
pub fn domain_deepen_overview(present_packs: &[String], fields: &Value) -> Value {
    let profile = classify_probe_stack_profile(fields);
    let present: std::collections::HashSet<&str> =
        present_packs.iter().map(|s| s.as_str()).collect();
    let domains: Vec<Value> = PROBE_DOMAINS
        .iter()
        .map(|d| {
            let primary_n = d
                .primary_packs
                .iter()
                .filter(|p| present.contains(*p))
                .count();
            let deepen_n = d
                .deepen_packs
                .iter()
                .filter(|p| present.contains(*p))
                .count();
            json!({
                "id": d.id,
                "primary_done": primary_n,
                "primary_total": d.primary_packs.len(),
                "deepen_ticks": deepen_n,
                "max_deepen_ticks": d.max_deepen_ticks,
                "serial_class": d.serial_class,
                "axes": d.axes,
            })
        })
        .collect();
    json!({
        "algo": "domain_deepen_overview_v1",
        "stack_profile": profile,
        "domains": domains,
        "policy": "Do not invent B*x for every batch; residual is the only B*x family",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn always_open_b10x_even_when_measurement_complete() {
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "webkit",
            "residual_probe_engine": "webkit",
            "engine_claim": "webkit",
            "engine_obs": "webkit",
            "browser_name": "Epiphany",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Epiphany/46.5",
            "webgl_unmasked_vendor": "Apple Inc.",
            "webgl_unmasked_renderer": "",
        });
        let p = classify_probe_stack_profile(&fields);
        assert_eq!(p["sample"]["status"], json!("rare_native"));
        // Complete → still open B10x
        let d = residual_deepen_decision(&fields, true, true, 0, 6, false);
        assert_eq!(d["worth_deepen"], json!(true));
        assert_eq!(d["deepen_reason"], json!("always_open_b10x_after_b10"));
        assert_eq!(d["stack_profile_schedule_gate"], json!(false));
        // Incomplete → also open
        let d2 = residual_deepen_decision(&fields, false, true, 0, 6, false);
        assert_eq!(d2["worth_deepen"], json!(true));
        assert_eq!(
            d2["deepen_reason"],
            json!("always_open_b10x_contract_also_incomplete")
        );
        // Exhausted ticks → stop
        let d3 = residual_deepen_decision(&fields, true, true, 6, 6, false);
        assert_eq!(d3["worth_deepen"], json!(false));
    }

    #[test]
    fn claim_ne_obs_is_stack_anomaly() {
        let fields = json!({
            "engine_family": "gecko",
            "engine_claim": "blink",
            "engine_obs": "gecko",
            "user_agent": "Mozilla/5.0 (Windows NT 10.0) Chrome/120.0.0.0",
            "platform": "Linux x86_64",
        });
        let p = classify_probe_stack_profile(&fields);
        assert_eq!(p["stack_anomaly"], json!(true));
        let prefs = p["preferred_domains"].as_array().unwrap();
        assert!(prefs.iter().any(|x| x.as_str() == Some("browser_kernel")));
    }

    #[test]
    fn blink_also_always_open_b10x_after_b10() {
        let fields = json!({
            "engine_family": "blink",
            "engine_claim": "blink",
            "engine_obs": "blink",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
        });
        // Always open for blink too (not only webkit).
        let d = residual_deepen_decision(&fields, true, true, 0, 6, false);
        assert_eq!(d["worth_deepen"], json!(true));
        assert_eq!(d["deepen_reason"], json!("always_open_b10x_after_b10"));
        let d2 = residual_deepen_decision(&fields, false, true, 0, 6, false);
        assert_eq!(d2["worth_deepen"], json!(true));
    }

    #[test]
    fn domain_table_has_residual_b10x_only() {
        let residual = domain_by_id("device_residual").unwrap();
        assert!(residual.deepen_packs.iter().all(|p| p.starts_with("B10x_")));
        let browser = domain_by_id("browser_kernel").unwrap();
        assert!(browser.deepen_packs.iter().all(|p| !p.starts_with("B10x_")));
    }
}
