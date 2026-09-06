//! iss/opus5 03-P2-6: bot verdict thresholds + per-flag rule weights + stack_auth
//! spoof/vm increments externalized to a **signed** policy spec
//! (`spec/bot_scoring_weights_v1.json`).
//!
//! - The shipped spec file mirrors the built-in defaults exactly (a unit test
//!   guards drift). Operators may ship an updated spec without recompiling.
//! - Optional Ed25519 signature: put base64 sig in the `sig` key and set
//!   `GR_BOT_WEIGHTS_PUBKEY` (hex or base64, 32B). Signature bind is the
//!   canonical JSON (serde_json map serialization is key-sorted → deterministic).
//!   A bad signature is **fail-closed**: built-in defaults keep running and the
//!   source is reported as `sig_rejected`, never a silently adopted file.
//! - Shadow evaluation (`GR_BOT_WEIGHTS_SHADOW=1`): the verdict keeps using
//!   the built-in (old) weights while the candidate spec is computed and only
//!   *recorded* in the bot details — A/B without effect, per the audit.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

pub const WEIGHTS_ALGO: &str = "bot_scoring_weights_v1";

#[derive(Debug, Clone)]
pub struct FlagW {
    pub baseline: i32,
    pub balanced: i32,
    /// balanced variant when env_headless || headless_ua (swiftshader rule).
    pub balanced_headless: Option<i32>,
    /// balanced variant when NOT env_headless (swiftshader plain rule).
    pub balanced_plain: Option<i32>,
}

impl Default for FlagW {
    fn default() -> Self {
        Self {
            baseline: 0,
            balanced: 0,
            balanced_headless: None,
            balanced_plain: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BotWeights {
    pub source: String,
    pub version: String,
    pub threshold_crawler: i32,
    pub threshold_bot: i32,
    pub threshold_suspect: i32,
    pub threshold_watch: i32,
    pub flags: HashMap<String, FlagW>,
    /// stack_auth increment code → [spoof, vm].
    pub stack_increments: HashMap<String, [f64; 2]>,
}

impl Default for BotWeights {
    fn default() -> Self {
        Self::builtin()
    }
}

impl BotWeights {
    /// The shipped spec's default table (must stay equal to spec/*.json; the
    /// `spec_matches_builtin_defaults` test guards drift).
    pub fn builtin() -> Self {
        let mut flags = HashMap::new();
        let mut f = |name: &str, base: i32, bal: i32| {
            flags.insert(name.to_string(), FlagW {
                baseline: base,
                balanced: bal,
                balanced_headless: None,
                balanced_plain: None,
            });
        };
        f("webdriver_true", 35, 35);
        f("webdriver_descriptor_tampered", 22, 22);
        f("playwright_marker", 30, 30);
        f("selenium_or_cdc", 30, 30);
        f("phantom_marker", 28, 28);
        f("headless_ua", 30, 30);
        f("session_mode_headless", 20, 18);
        f("crawler_bot_ua", 40, 40);
        f("outer_window_zero", 15, 15);
        f("gpu_label_spoof_or_soft_residual", 18, 18);
        f("soft_stack_residual", 12, 12);
        f("chrome_runtime_missing", 0, 10);
        f("webgl_missing_while_chrome", 0, 8);
        f("high_cores_small_screen", 0, 12);
        f("extreme_hardware_concurrency", 0, 5);
        f("en_only_on_zh_site", 5, 2);
        f("utc_timezone_on_zh_site", 6, 3);
        flags.insert("gpu_swiftshader_or_software".to_string(), FlagW {
            baseline: 22,
            balanced: 20,
            balanced_headless: None,
            balanced_plain: Some(10),
        });
        let mut stack = HashMap::new();
        let mut s = |k: &str, sp: f64, vm: f64| {
            stack.insert(k.to_string(), [sp, vm]);
        };
        s("noise_injection_likely", 0.18, 0.0);
        s("too_stable_with_label_conflict", 0.12, 0.0);
        s("challenge_unique_n1_soft_label", 0.10, 0.0);
        s("main_residual_hist_soft_shape", 0.15, 0.0);
        s("gpu_slope_soft_like", 0.14, 0.08);
        s("caps_claim_vs_actual", 0.28, 0.0);
        s("precision_claim_vs_behavior", 0.16, 0.0);
        s("native_integrity_low", 0.22, 0.0);
        s("non_native_count_high", 0.12, 0.0);
        s("clock_coarse_resolution", 0.0, 0.12);
        s("raf_jitter_high", 0.0, 0.08);
        s("date_now_skew_high", 0.0, 0.10);
        s("eme_all_unsupported", 0.0, 0.10);
        s("codec_matrix_empty_virt_hint", 0.0, 0.12);
        s("eme_all_unsupported_depth", 0.0, 0.08);
        s("gpu_ns_wall_ratio_inconsistent_v1", 0.15, 0.0);
        s("gpu_ns_wall_ratio_inconsistent_v2", 0.12, 0.0);
        s("gpu_disjoint_rate_high", 0.0, 0.10);
        s("challenge_seed_invalid", 0.20, 0.0);
        s("agent_parity_automation_globals", 0.25, 0.0);
        s("storage_heavily_restricted", 0.0, 0.05);
        s("layer_divergence_high", 0.12, 0.0);
        s("layer_divergence_mismatch", 0.10, 0.0);
        s("thermal_cpu_slope_high", 0.0, 0.08);
        s("ws_constructor_not_native", 0.10, 0.0);
        s("gpu_r2_ns_low", 0.10, 0.0);
        s("gpu_ns_non_monotonic", 0.08, 0.0);
        s("gpu_ns_depth_weak", 0.06, 0.0);
        s("thermal_full_slope_high", 0.0, 0.10);
        s("protocol_tcp_quic_mismatch", 0.08, 0.0);
        s("challenge_eval_failed", 0.10, 0.0);
        s("canvas_geometry_unstable", 0.06, 0.0);
        s("agent_parity_ratio_low", 0.12, 0.0);
        s("residual_challenge_delta_high", 0.12, 0.0);
        s("caps_claim_vs_actual_gap_high", 0.18, 0.0);
        s("raf_mean_coarse", 0.0, 0.08);
        s("thermal_gpu_slope_full_high", 0.0, 0.08);
        s("api_flags_sparse", 0.08, 0.0);
        s("caps_internal_inconsistent", 0.14, 0.0);
        s("automation_globals_nonempty", 0.18, 0.0);
        s("codec_mostly_empty", 0.0, 0.08);
        s("sandbox_main_residual_mismatch", 0.10, 0.0);
        s("screen_color_depth_low", 0.05, 0.0);
        s("thermal_cpu_cv_high", 0.0, 0.05);
        s("tcp_retrans_high", 0.0, 0.04);
        s("webgl_label_vs_residual", 0.45, 0.0);
        s("webgl_label_vs_soft", 0.25, 0.0);
        s("caps_claim_vs_soft_behavior", 0.15, 0.0);
        s("stack_soft_render", 0.0, 0.25);
        s("vm_virt_gpu", 0.0, 0.35);
        Self {
            source: "builtin".into(),
            version: "1.0.0".into(),
            threshold_crawler: 40,
            threshold_bot: 65,
            threshold_suspect: 45,
            threshold_watch: 25,
            flags,
            stack_increments: stack,
        }
    }

    pub fn flag_pts(&self, algo: &str, flag: &str, env_headless: bool) -> i32 {
        let Some(w) = self.flags.get(flag) else {
            return 0;
        };
        match algo {
            "baseline" => w.baseline,
            "balanced" => {
                if env_headless {
                    w.balanced_headless.unwrap_or(w.balanced)
                } else {
                    w.balanced_plain.unwrap_or(w.balanced)
                }
            }
            _ => 0,
        }
    }

    pub fn stack_inc(&self, code: &str, d_spoof: f64, d_vm: f64) -> (f64, f64) {
        match self.stack_increments.get(code) {
            Some([sp, vm]) => (*sp, *vm),
            None => (d_spoof, d_vm),
        }
    }

    /// Verdict thresholds in (crawler_floor, bot, suspect, watch) order.
    pub fn thresholds(&self) -> (i32, i32, i32, i32) {
        (
            self.threshold_crawler,
            self.threshold_bot,
            self.threshold_suspect,
            self.threshold_watch,
        )
    }
}

fn parse_weights(v: &Value, source: &str) -> BotWeights {
    let mut w = BotWeights::builtin();
    w.source = source.to_string();
    w.version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("1.0.0")
        .to_string();
    if let Some(t) = v.get("thresholds") {
        let num = |k: &str, d: i32| t.get(k).and_then(|x| x.as_i64()).map(|n| n as i32).unwrap_or(d);
        w.threshold_crawler = num("crawler_floor", 40);
        w.threshold_bot = num("bot", 65);
        w.threshold_suspect = num("suspect", 45);
        w.threshold_watch = num("watch", 25);
    }
    if let Some(fl) = v.get("flags").and_then(|x| x.as_object()) {
        for (name, spec) in fl {
            let entry = match spec {
                Value::Number(n) => {
                    let p = n.as_i64().unwrap_or(0) as i32;
                    FlagW {
                        baseline: p,
                        balanced: p,
                        balanced_headless: None,
                        balanced_plain: None,
                    }
                }
                Value::Object(o) => {
                    let num = |k: &str, d: i32| o.get(k).and_then(|x| x.as_i64()).map(|n| n as i32).unwrap_or(d);
                    FlagW {
                        baseline: num("baseline", 0),
                        balanced: num("balanced", 0),
                        balanced_headless: o.get("balanced_headless").and_then(|x| x.as_i64()).map(|n| n as i32),
                        balanced_plain: o.get("balanced_plain").and_then(|x| x.as_i64()).map(|n| n as i32),
                    }
                }
                _ => continue,
            };
            w.flags.insert(name.clone(), entry);
        }
    }
    if let Some(sa) = v.get("stack_auth").and_then(|x| x.as_object()) {
        for (code, spec) in sa {
            let vals = match spec {
                Value::Number(n) => {
                    let p = n.as_f64().unwrap_or(0.0);
                    [p, 0.0]
                }
                Value::Object(o) => [
                    o.get("spoof").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    o.get("vm").and_then(|x| x.as_f64()).unwrap_or(0.0),
                ],
                _ => continue,
            };
            w.stack_increments.insert(code.clone(), vals);
        }
    }
    w
}

fn decode_pubkey(raw: &str) -> Option<[u8; 32]> {
    let raw = raw.trim();
    let bytes = if raw.len() == 64 {
        // hex
        (0..32).map(|i| u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16).ok()).collect::<Option<Vec<u8>>>()
    } else {
        // base64
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(raw).ok()
    }?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

/// Verify `sig` (base64) over the canonical JSON (key-sorted) with the
/// `GR_BOT_WEIGHTS_PUBKEY`. `None` = no signature/public key present.
fn verify_signature(v: &Value) -> Result<(), String> {
    let Some(sig_b64) = v.get("sig").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let Some(pubkey_raw) = gr_abi::env::get("BOT_WEIGHTS_PUBKEY") else {
        return Err("sig present but GR_BOT_WEIGHTS_PUBKEY unset — refusing unsigned override".into());
    };
    let pk_bytes = decode_pubkey(&pubkey_raw)
        .ok_or_else(|| String::from("GR_BOT_WEIGHTS_PUBKEY invalid (need 32B hex/base64)"))?;
    let vk = VerifyingKey::from_bytes(&pk_bytes).map_err(|e| e.to_string())?;
    let mut canonical = v.clone();
    if let Some(o) = canonical.as_object_mut() {
        o.remove("sig");
    }
    let msg = serde_json::to_string(&canonical).map_err(|e| e.to_string())?;
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .map_err(|_| "sig not base64".to_string())?;
    if raw.len() != 64 {
        return Err("sig must be 64 bytes".into());
    }
    let mut sb = [0u8; 64];
    sb.copy_from_slice(&raw);
    vk.verify(msg.as_bytes(), &Signature::from_bytes(&sb))
        .map_err(|_| "ed25519 signature mismatch".to_string())
}

fn spec_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = gr_abi::env::get("BOT_WEIGHTS_SPEC") {
        if !p.trim().is_empty() {
            paths.push(std::path::PathBuf::from(p.trim()));
        }
    }
    paths.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/bot_scoring_weights_v1.json"));
    paths.push(std::path::PathBuf::from("spec/bot_scoring_weights_v1.json"));
    paths
}

fn load() -> BotWeights {
    for p in spec_paths() {
        let Ok(raw) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if v.get("weights_algo").and_then(|x| x.as_str()) != Some(WEIGHTS_ALGO) {
            continue;
        }
        match verify_signature(&v) {
            Ok(()) => {
                let signed = v.get("sig").is_some();
                return parse_weights(&v, if signed { "signed_spec" } else { "spec_file" });
            }
            Err(e) => {
                let mut w = BotWeights::builtin();
                w.source = format!("sig_rejected::{e}");
                w.version = "builtin-fallback".into();
                return w;
            }
        }
    }
    BotWeights::builtin()
}

fn adopted() -> &'static BotWeights {
    static ADOPTED: OnceLock<BotWeights> = OnceLock::new();
    ADOPTED.get_or_init(load)
}

/// Shadow A/B: when `GR_BOT_WEIGHTS_SHADOW=1`, the *verdict* keeps the old
/// (builtin) weights and the adopted spec is computed record-only. When off,
/// the adopted spec is the active scorer.
pub fn active() -> &'static BotWeights {
    if shadow_mode() {
        static BUILTIN_REF: OnceLock<BotWeights> = OnceLock::new();
        BUILTIN_REF.get_or_init(BotWeights::builtin)
    } else {
        adopted()
    }
}

/// Candidate weights for the shadow record (the adopted spec/builtin).
pub fn candidate() -> &'static BotWeights {
    adopted()
}

pub fn shadow_mode() -> bool {
    gr_abi::env::get("BOT_WEIGHTS_SHADOW")
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Verdict for a score with a given weights table (band() parity for shadow).
pub fn verdict_with(w: &BotWeights, score: i32, flags: &[String]) -> (String, String) {
    let (cf, tb, ts, tw) = w.thresholds();
    if flags.iter().any(|x| x == "crawler_bot_ua") && score >= cf {
        ("crawler".into(), "crawler".into())
    } else if score >= tb {
        ("bot".into(), "max_fam".into())
    } else if score >= ts {
        ("suspect".into(), "max_fam".into())
    } else if score >= tw {
        ("watch".into(), "max_fam".into())
    } else {
        ("human".into(), "human".into())
    }
}

/// Transparency meta attached to every BotScore.details.
pub fn meta_value() -> Value {
    let w = active();
    json!({
        "weights_algo": WEIGHTS_ALGO,
        "weights_source": w.source,
        "weights_version": w.version,
        "shadow": shadow_mode(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_matches_builtin_defaults() {
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../spec/bot_scoring_weights_v1.json"),
            )
            .expect("spec file"),
        )
        .expect("spec is valid json");
        let parsed = parse_weights(&v, "spec_file");
        let builtin = BotWeights::builtin();
        assert_eq!(parsed.threshold_crawler, builtin.threshold_crawler);
        assert_eq!(parsed.threshold_bot, builtin.threshold_bot);
        assert_eq!(parsed.threshold_suspect, builtin.threshold_suspect);
        assert_eq!(parsed.threshold_watch, builtin.threshold_watch);
        assert_eq!(
            parsed.flags.len(),
            builtin.flags.len(),
            "flag table drift against spec"
        );
        for (k, fw) in &builtin.flags {
            let p = parsed.flags.get(k).unwrap_or_else(|| panic!("spec missing flag {k}"));
            assert_eq!(p.baseline, fw.baseline, "flag {k} baseline drift");
            assert_eq!(p.balanced, fw.balanced, "flag {k} balanced drift");
            assert_eq!(p.balanced_headless, fw.balanced_headless, "flag {k} headless drift");
            assert_eq!(p.balanced_plain, fw.balanced_plain, "flag {k} plain drift");
        }
        assert_eq!(parsed.stack_increments.len(), builtin.stack_increments.len());
        for (k, v) in &builtin.stack_increments {
            let p = parsed.stack_increments.get(k).unwrap_or_else(|| panic!("spec missing stack_inc {k}"));
            assert_eq!(p, v, "stack_inc {k} drift");
        }
    }

    #[test]
    fn flag_pts_variants() {
        let w = BotWeights::builtin();
        assert_eq!(w.flag_pts("baseline", "webdriver_true", false), 35);
        assert_eq!(w.flag_pts("balanced", "webdriver_true", false), 35);
        // swiftshader: env_headless → 20, else 10 (balanced)
        assert_eq!(
            w.flag_pts("balanced", "gpu_swiftshader_or_software", true),
            20
        );
        assert_eq!(
            w.flag_pts("balanced", "gpu_swiftshader_or_software", false),
            10
        );
        assert_eq!(
            w.flag_pts("baseline", "gpu_swiftshader_or_software", true),
            22
        );
        // balanced-only flags are 0 in baseline
        assert_eq!(w.flag_pts("baseline", "chrome_runtime_missing", false), 0);
        assert_eq!(w.flag_pts("balanced", "chrome_runtime_missing", false), 10);
        // unknown flag → 0
        assert_eq!(w.flag_pts("balanced", "nope", false), 0);
    }

    #[test]
    fn stack_inc_override_and_default() {
        let w = BotWeights::builtin();
        assert_eq!(w.stack_inc("webgl_label_vs_residual", 0.1, 0.0), (0.45, 0.0));
        assert_eq!(w.stack_inc("gpu_slope_soft_like", 0.0, 0.0), (0.14, 0.08));
        // unknown code falls back to the caller's default
        assert_eq!(w.stack_inc("no_such_code", 0.33, 0.22), (0.33, 0.22));
    }

    #[test]
    fn verdict_thresholds() {
        let w = BotWeights::builtin();
        let no_flags = || Vec::<String>::new();
        assert_eq!(verdict_with(&w, 10, &no_flags()).0, "human");
        assert_eq!(verdict_with(&w, 30, &no_flags()).0, "watch");
        assert_eq!(verdict_with(&w, 50, &no_flags()).0, "suspect");
        assert_eq!(verdict_with(&w, 80, &no_flags()).0, "bot");
        // crawler flag + floor
        assert_eq!(
            verdict_with(&w, 42, &["crawler_bot_ua".to_string()]).0,
            "crawler"
        );
        assert_eq!(
            verdict_with(&w, 30, &["crawler_bot_ua".to_string()]).0,
            "watch"
        );
    }

    #[test]
    fn spec_parse_accepts_number_flags() {
        let v = json!({
            "weights_algo": WEIGHTS_ALGO,
            "flags": {"webdriver_true": 35},
            "stack_auth": {"noise_injection_likely": 0.18}
        });
        let w = parse_weights(&v, "spec_file");
        assert_eq!(w.flag_pts("balanced", "webdriver_true", false), 35);
        assert_eq!(w.flag_pts("baseline", "webdriver_true", false), 35);
        assert_eq!(w.stack_inc("noise_injection_likely", 0.0, 0.0), (0.18, 0.0));
    }
}
