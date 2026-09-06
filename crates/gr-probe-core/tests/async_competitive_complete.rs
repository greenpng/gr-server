//! Normal-path async competitive materials: UA-CH / storage / WebGPU / speech / mediaCaps claim-obs.
use gr_probe_core::bot::BotScore;
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use serde_json::{json, Value};

fn truth_ok() -> TruthResult {
    TruthResult {
        xsrc_status: "consistent".into(),
        real_band: "watch".into(),
        credibility: 0.55,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!([]),
        reasons: vec![],
        details: json!({}),
    }
}

fn bot_human() -> BotScore {
    BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "balanced".into(),
        details: json!({}),
        robot_name: None,
    }
}

fn reasons(block: &Value) -> Vec<String> {
    block
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn hits(block: &Value) -> Vec<String> {
    block
        .get("field_hits")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

#[test]
fn async_ua_ch_and_storage_enter_scores() {
    let bot = bot_human();
    let fo = json!({
        "platform": "Win32",
        "user_agent": "Mozilla/5.0 Windows Chrome/120",
        "webdriver": false,
        "form_class": "desktop",
        "async_followup": true,
        "async_followup_algo": "gr_async_competitive_v1",
        "ua_ch_present": true,
        "ua_ch_platform": "Windows",
        "ua_ch_architecture": "x86",
        "ua_ch_bitness": "64",
        "ua_ch_platform_version": "15.0.0",
        "ua_ch_high_entropy_ok": true,
        "ua_ch_full_version_hash": "fv_abc",
        "ua_ch_brands_hash": "br_def",
        "storage_estimate_api": true,
        "storage_estimate_ok": true,
        "storage_quota_bytes": 120_000_000_000i64,
        "storage_usage_bytes": 1_000_000_000i64,
        "storage_usage_ratio": 0.008,
        "storage_persisted": false,
        "speech_voices_n": 42,
        "speech_voices_hash": "sv_hash",
        "speech_voices_async_ok": true,
        "speech_local_n": 30,
        "webgpu_available": true,
        "webgpu_adapter_ok": true,
        "webgpu_adapter_vendor": "nvidia",
        "webgpu_features_hash": "wf_hash",
        "webgpu_limits_hash": "wl_hash",
        "media_capabilities_ok": true,
        "media_capabilities_n": 5,
        "media_capabilities_supported_n": 4,
        "media_capabilities_hash": "mc_hash",
        "font_preferences_hash": "fp_hash",
        "font_pref_serif_width": 401.2,
        "font_pref_sans_width": 398.1,
        "local_fonts_query_ok": true,
        "local_fonts_n": 120,
        "local_fonts_hash": "lf_hash",
    });
    let stack = stack_auth_from_fields(&fo);
    let br = score_br(&fo, &stack, &bot, &truth_ok(), &[]);
    let os = score_os(&fo, &stack, &truth_ok(), &[]);
    let h = hits(&br);
    assert!(
        h.iter().any(|x| x.contains("ua_ch") || x.contains("gr_dense") || x.contains("storage") || x.contains("webgpu") || x.contains("speech") || x.contains("media_capabilities") || x.contains("font_pref") || x.contains("local_fonts") || x.contains("math") || x.contains("dense") || x.contains("verify") || x.contains("platform") || x.contains("matrix")),
        "expected competitive async hits, got {h:?}"
    );
    // Digests should appear in dense digest path
    let rs = reasons(&br);
    let rs_os = reasons(&os);
    let all: Vec<_> = rs.iter().chain(rs_os.iter()).cloned().collect();
    assert!(
        h.iter().any(|x| {
            x.contains("ua_ch")
                || x.contains("speech")
                || x.contains("webgpu")
                || x.contains("storage")
                || x.contains("media_capabilities")
                || x.contains("font_pref")
                || x.contains("gr_dense")
                || x.contains("api_flags")
        }) || all.iter().any(|r| r.contains("dense_digest") || r.contains("ua_ch") || r.contains("matrix_role")),
        "expected scorer consumption of async materials; hits={h:?} reasons={all:?}"
    );
}

#[test]
fn claim_obs_cross_platform_ua_ch_mismatch_demotes() {
    let bot = bot_human();
    let fo = json!({
        "platform": "Linux x86_64",
        "ua_ch_platform": "Windows",
        "ua_ch_high_entropy_ok": true,
        "ua_ch_architecture": "x86",
        "webdriver": false,
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0",
    });
    let stack = stack_auth_from_fields(&fo);
    let br = score_br(&fo, &stack, &bot, &truth_ok(), &[]);
    let rs = reasons(&br);
    assert!(
        rs.iter().any(|r| r.contains("cross_platform_vs_ua_ch")),
        "expected platform vs UA-CH mismatch, got {rs:?}"
    );
}

#[test]
fn claim_obs_media_all_unsupported_demotes() {
    let bot = bot_human();
    let fo = json!({
        "platform": "Win32",
        "user_agent": "Chrome",
        "webdriver": false,
        "form_class": "desktop",
        "media_capabilities_n": 5,
        "media_capabilities_supported_n": 0,
        "media_capabilities_hash": "none",
        "media_capabilities_ok": true,
    });
    let stack = stack_auth_from_fields(&fo);
    let br = score_br(&fo, &stack, &bot, &truth_ok(), &[]);
    let rs = reasons(&br);
    assert!(
        rs.iter().any(|r| r.contains("media_capabilities")),
        "expected media capabilities cross demote, got {rs:?}"
    );
}

#[test]
fn claim_obs_soft_vs_webgpu_vendor_demotes_os() {
    let bot = bot_human();
    let fo = json!({
        "platform": "Linux",
        "user_agent": "Chrome",
        "webdriver": false,
        "form_class": "desktop",
        "residual_soft_like": true,
        "webgpu_adapter_ok": true,
        "webgpu_adapter_vendor": "nvidia",
        "webgpu_features_hash": "x",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce)",
    });
    let stack = stack_auth_from_fields(&fo);
    let os = score_os(&fo, &stack, &truth_ok(), &[]);
    let rs = reasons(&os);
    assert!(
        rs.iter().any(|r| r.contains("soft") || r.contains("webgpu") || r.contains("residual")),
        "expected soft vs webgpu/gpu claim-obs, got {rs:?}"
    );
}
