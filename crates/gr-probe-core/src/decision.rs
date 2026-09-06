//! Product decision table: bot ∥ identity ∥ Soft 旁证 (never promote).
//!
//! Order (iss/02 · iss/09):
//!   1. Bot veto (crawler/bot)
//!   2. Presence of commercial `dv_*`
//!   3. Soft as 旁证 only (`promote=false`)
//!   4. Sensitive vs non-sensitive action policy
//!
//! Confidence is always labeled `heuristic_v0` unless caller upgrades version.

use crate::policy::{load_product_policy, otp_thresholds};
use crate::soft_v2::PROMOTE_TO_COMMERCIAL_ID;
use serde_json::{json, Value};

/// Confidence semantic version exposed to product (not calibrated 1−P_fp).
pub const CONFIDENCE_VERSION: &str = "heuristic_v0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionSensitivity {
    NonSensitive,
    Sensitive,
}

impl ActionSensitivity {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "sensitive" | "high" | "withdraw" | "bind" | "payment" => Self::Sensitive,
            _ => Self::NonSensitive,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionInput {
    pub bot_verdict: String,
    pub device_id: Option<String>,
    pub soft_edge: bool,
    pub soft_promote: bool,
    pub confidence: f64,
    pub confidence_version: String,
    pub sensitivity: ActionSensitivity,
    /// Session os safety score 0..1 (higher = safer). Optional; None skips L4 os gate.
    pub os_score: Option<f64>,
    /// Session br safety score 0..1.
    pub br_score: Option<f64>,
    /// Page rpa safety score 0..1.
    pub rpa_score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionOutcome {
    pub allow: bool,
    pub require_otp: bool,
    pub require_step_up: bool,
    pub use_device_id_as_primary: bool,
    pub use_soft_as_旁证: bool,
    pub anonymous: bool,
    pub bot_veto: bool,
    pub policy: String,
    pub reasons: Vec<String>,
    pub soft_promote_forced_false: bool,
    pub confidence_version: String,
}

/// Pure ordered product policy. Soft never becomes commercial primary.
pub fn decide_product_action(input: &DecisionInput) -> DecisionOutcome {
    let mut reasons = Vec::new();
    let bot = input.bot_verdict.to_ascii_lowercase();
    let bot_veto = matches!(bot.as_str(), "bot" | "crawler" | "automation");
    // Constitution: promote flag is always false for product decisions.
    let soft_promote_forced_false = !input.soft_promote && !PROMOTE_TO_COMMERCIAL_ID
        || PROMOTE_TO_COMMERCIAL_ID == false;
    let _ = soft_promote_forced_false;
    let conf_ver = if input.confidence_version.is_empty() {
        CONFIDENCE_VERSION.to_string()
    } else {
        input.confidence_version.clone()
    };

    if bot_veto {
        reasons.push("bot_veto".into());
        return DecisionOutcome {
            allow: false,
            require_otp: true,
            require_step_up: true,
            use_device_id_as_primary: false,
            use_soft_as_旁证: false,
            anonymous: true,
            bot_veto: true,
            policy: "deny_bot".into(),
            reasons,
            soft_promote_forced_false: true,
            confidence_version: conf_ver,
        };
    }

    // Multi-segment product ids (dv0-|dv4-|dv5-|dv6-) and legacy exclusive commercial ids.
    let has_dv = input
        .device_id
        .as_ref()
        .map(|s| crate::device_tier::is_commercial_device_id(s) && s.len() > 4)
        .unwrap_or(false);

    // L4 policy gates (G-ARCH-13/15): configurable OTP thresholds — not digest redlines.
    let policy = load_product_policy();
    let (br_lt, os_lt, rpa_lt, conf_lt) = otp_thresholds(policy);
    let mut score_otp = false;
    if let Some(br) = input.br_score {
        if br < br_lt {
            reasons.push(format!("br_score_lt_policy:{br:.3}<{br_lt}"));
            score_otp = true;
        }
    }
    if let Some(os) = input.os_score {
        if os < os_lt {
            reasons.push(format!("os_score_lt_policy:{os:.3}<{os_lt}"));
            score_otp = true;
        }
    }
    if let Some(rpa) = input.rpa_score {
        if rpa < rpa_lt {
            reasons.push(format!("rpa_score_lt_policy:{rpa:.3}<{rpa_lt}"));
            score_otp = true;
        }
    }

    if has_dv {
        reasons.push("has_commercial_device_id".into());
        let sensitive = input.sensitivity == ActionSensitivity::Sensitive;
        let low_conf = input.confidence < conf_lt;
        if (sensitive && low_conf) || score_otp {
            if sensitive && low_conf {
                reasons.push("sensitive_low_confidence_step_up".into());
            }
            if score_otp {
                reasons.push("product_score_otp_policy".into());
            }
            return DecisionOutcome {
                allow: true,
                require_otp: true,
                require_step_up: true,
                use_device_id_as_primary: true,
                use_soft_as_旁证: input.soft_edge,
                anonymous: false,
                bot_veto: false,
                policy: "allow_dv_step_up".into(),
                reasons,
                soft_promote_forced_false: true,
                confidence_version: conf_ver,
            };
        }
        return DecisionOutcome {
            allow: true,
            require_otp: false,
            require_step_up: false,
            use_device_id_as_primary: true,
            use_soft_as_旁证: input.soft_edge,
            anonymous: false,
            bot_veto: false,
            policy: if sensitive {
                "allow_dv_sensitive".into()
            } else {
                "allow_dv".into()
            },
            reasons,
            soft_promote_forced_false: true,
            confidence_version: conf_ver,
        };
    }

    // No commercial id
    reasons.push("no_commercial_device_id".into());
    if input.soft_edge {
        reasons.push("soft_旁证_only".into());
    }
    // Soft must never bind as primary even if edge present
    if input.soft_promote || PROMOTE_TO_COMMERCIAL_ID {
        reasons.push("soft_promote_rejected".into());
    }

    match input.sensitivity {
        ActionSensitivity::Sensitive => {
            reasons.push("sensitive_requires_otp_or_deny_primary".into());
            DecisionOutcome {
                allow: true, // allow session but not primary bind
                require_otp: true,
                require_step_up: true,
                use_device_id_as_primary: false,
                use_soft_as_旁证: input.soft_edge,
                anonymous: true,
                bot_veto: false,
                policy: "anonymous_sensitive_otp".into(),
                reasons,
                soft_promote_forced_false: true,
                confidence_version: conf_ver,
            }
        }
        ActionSensitivity::NonSensitive => DecisionOutcome {
            allow: true,
            require_otp: score_otp,
            require_step_up: score_otp,
            use_device_id_as_primary: false,
            use_soft_as_旁证: input.soft_edge,
            anonymous: true,
            bot_veto: false,
            policy: if score_otp {
                "anonymous_score_otp".into()
            } else {
                "anonymous_nonsensitive".into()
            },
            reasons,
            soft_promote_forced_false: true,
            confidence_version: conf_ver,
        },
    }
}

impl DecisionOutcome {
    pub fn to_value(&self) -> Value {
        json!({
            "allow": self.allow,
            "require_otp": self.require_otp,
            "require_step_up": self.require_step_up,
            "use_device_id_as_primary": self.use_device_id_as_primary,
            "use_soft_as_corroboration": self.use_soft_as_旁证,
            "anonymous": self.anonymous,
            "bot_veto": self.bot_veto,
            "policy": self.policy,
            "reasons": self.reasons,
            "soft_promote": false,
            "soft_promote_forced_false": self.soft_promote_forced_false,
            "confidence_version": self.confidence_version,
            "decision_order": ["bot_veto", "has_dv", "soft_旁证", "sensitivity"],
        })
    }
}

/// iss/36 D-2 · atlas E-13: business-readable degradation action (not a digest decision).
/// Values: `allow` | `allow_soft` | `step_up` | `deny_sensitive` | `challenge_bot`.
pub fn derive_recommended_action(
    product: &Value,
    device: &Value,
    bot_verdict: &str,
    stop_reason: Option<&str>,
    envelope_in_range: Option<bool>,
) -> Value {
    let mut reasons: Vec<String> = Vec::new();
    let bot = bot_verdict.to_ascii_lowercase();
    if matches!(bot.as_str(), "bot" | "crawler") {
        reasons.push("bot_verdict".into());
        return json!({
            "action": "challenge_bot",
            "reasons": reasons,
            "stop_reason": stop_reason,
            "schema": "gr_recommended_action_v1",
        });
    }

    let collision = device
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .or_else(|| product.get("collision_risk").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let soft = device
        .pointer("/trust/soft_stack")
        .and_then(|v| v.as_bool())
        .or_else(|| device.get("soft_stack").and_then(|v| v.as_bool()))
        .unwrap_or(false)
        || device
            .get("digest_path")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("soft"))
            .unwrap_or(false);
    let level = device
        .get("association_level")
        .or_else(|| product.get("association_level"))
        .and_then(|v| v.as_str())
        .unwrap_or("gateway");
    let posture = product
        .pointer("/device_confidence_posture")
        .or_else(|| product.pointer("/os/confidence_report/posture"))
        .or_else(|| product.get("confidence_posture"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let os_s = product.pointer("/os/score").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let br_s = product.pointer("/br/score").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let rpa_s = product.pointer("/rpa/score").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let out_env = envelope_in_range == Some(false);
    let stop = stop_reason.unwrap_or("");

    if out_env {
        reasons.push("out_of_envelope".into());
    }
    if collision {
        reasons.push("collision_risk".into());
    }
    if soft || matches!(level, "env" | "profile" | "gateway") {
        if soft {
            reasons.push("soft_or_env_bind".into());
        } else {
            reasons.push(format!("association_level={level}"));
        }
    }
    if posture == "thin" || posture == "partial" {
        reasons.push(format!("posture={posture}"));
    }
    if os_s < 0.40 || br_s < 0.40 || rpa_s < 0.40 {
        reasons.push("axis_score_low".into());
    }
    if !stop.is_empty() && stop != "coverage_complete" && stop != "probe_complete" {
        reasons.push(format!("stop_reason={stop}"));
    }

    // association_level=env is soft/VM bind — always allow_soft (not hard allow),
    // even when soft_stack flag is absent (separator-present env path).
    let action = if bot == "automation" || rpa_s < 0.28 {
        reasons.push("automation_or_rpa_floor".into());
        "step_up"
    } else if os_s < 0.32 && br_s < 0.32 {
        reasons.push("dual_axis_collapse".into());
        "deny_sensitive"
    } else if soft
        || collision
        || matches!(level, "env" | "profile" | "gateway")
        || out_env
    {
        "allow_soft"
    } else if posture == "thin" || os_s < 0.45 || br_s < 0.45 {
        "step_up"
    } else {
        "allow"
    };

    if reasons.is_empty() {
        reasons.push("default_allow".into());
    }

    json!({
        "action": action,
        "reasons": reasons,
        "stop_reason": stop_reason,
        "association_level": level,
        "collision_risk": collision,
        "schema": "gr_recommended_action_v1",
    })
}

/// Build decision input from evaluate-like device/bot/soft JSON slices.
pub fn decide_from_evaluate_parts(
    bot_verdict: &str,
    device_id: Option<&str>,
    soft_edge: bool,
    confidence: f64,
    sensitivity: &str,
) -> Value {
    decide_from_evaluate_parts_scored(
        bot_verdict,
        device_id,
        soft_edge,
        confidence,
        sensitivity,
        None,
        None,
        None,
    )
}

/// G-ARCH-15: include os/br/rpa safety scores for L4 OTP policy.
pub fn decide_from_evaluate_parts_scored(
    bot_verdict: &str,
    device_id: Option<&str>,
    soft_edge: bool,
    confidence: f64,
    sensitivity: &str,
    os_score: Option<f64>,
    br_score: Option<f64>,
    rpa_score: Option<f64>,
) -> Value {
    let outcome = decide_product_action(&DecisionInput {
        bot_verdict: bot_verdict.to_string(),
        device_id: device_id.map(|s| s.to_string()),
        soft_edge,
        soft_promote: PROMOTE_TO_COMMERCIAL_ID,
        confidence,
        confidence_version: CONFIDENCE_VERSION.into(),
        sensitivity: ActionSensitivity::parse(sensitivity),
        os_score,
        br_score,
        rpa_score,
    });
    outcome.to_value()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_scores() -> (Option<f64>, Option<f64>, Option<f64>) {
        (None, None, None)
    }

    #[test]
    fn bot_vetoes_first() {
        let (os, br, rpa) = empty_scores();
        let o = decide_product_action(&DecisionInput {
            bot_verdict: "crawler".into(),
            device_id: Some("dv_deadbeefdeadbeef".into()),
            soft_edge: true,
            soft_promote: false,
            confidence: 0.9,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::NonSensitive,
            os_score: os,
            br_score: br,
            rpa_score: rpa,
        });
        assert!(o.bot_veto);
        assert!(!o.allow);
        assert_eq!(o.policy, "deny_bot");
    }

    #[test]
    fn soft_never_primary_without_dv() {
        let (os, br, rpa) = empty_scores();
        let o = decide_product_action(&DecisionInput {
            bot_verdict: "human".into(),
            device_id: None,
            soft_edge: true,
            soft_promote: false,
            confidence: 0.5,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::Sensitive,
            os_score: os,
            br_score: br,
            rpa_score: rpa,
        });
        assert!(!o.use_device_id_as_primary);
        assert!(o.use_soft_as_旁证);
        assert!(o.require_otp);
        assert!(o.anonymous);
        assert!(!o.to_value()["soft_promote"].as_bool().unwrap());
    }

    #[test]
    fn has_dv_allows_primary() {
        let (os, br, rpa) = empty_scores();
        let o = decide_product_action(&DecisionInput {
            bot_verdict: "human".into(),
            device_id: Some("dv_0123456789abcdef".into()),
            soft_edge: false,
            soft_promote: false,
            confidence: 0.85,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::NonSensitive,
            os_score: os,
            br_score: br,
            rpa_score: rpa,
        });
        assert!(o.allow);
        assert!(o.use_device_id_as_primary);
        assert_eq!(o.confidence_version, CONFIDENCE_VERSION);
    }

    #[test]
    fn multi_segment_dv0_counts_as_commercial_primary() {
        let (os, br, rpa) = empty_scores();
        let multi = "dv0-0.26-abc123def0-1111111111-2222222222-linux-x86_64-c3-UTC-oihash000-rtchash00";
        let o = decide_product_action(&DecisionInput {
            bot_verdict: "human".into(),
            device_id: Some(multi.into()),
            soft_edge: false,
            soft_promote: false,
            confidence: 0.9,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::NonSensitive,
            os_score: os,
            br_score: br,
            rpa_score: rpa,
        });
        assert!(o.allow, "multi-segment must be commercial: {o:?}");
        assert!(
            o.use_device_id_as_primary,
            "dv0- multi mint must set use_device_id_as_primary: reasons={:?}",
            o.reasons
        );
        assert!(
            o.reasons.iter().any(|r| r == "has_commercial_device_id"),
            "expected has_commercial_device_id, got {:?}",
            o.reasons
        );
        assert_ne!(
            o.policy, "no_commercial_device_id",
            "must not treat multi as no_commercial: {o:?}"
        );
    }

    #[test]
    fn multi_segment_all_precision_lanes_are_commercial() {
        let (os, br, rpa) = empty_scores();
        for prefix in ["dv0-", "dv4-", "dv5-", "dv6-"] {
            let id = format!("{prefix}0.2600-wghash0000-auhash0000-cphash0000-linux-x86_64-c3-UTC-0-0");
            let o = decide_product_action(&DecisionInput {
                bot_verdict: "human".into(),
                device_id: Some(id.clone()),
                soft_edge: false,
                soft_promote: false,
                confidence: 0.88,
                confidence_version: CONFIDENCE_VERSION.into(),
                sensitivity: ActionSensitivity::NonSensitive,
                os_score: os,
                br_score: br,
                rpa_score: rpa,
            });
            assert!(
                o.use_device_id_as_primary,
                "{prefix} must be commercial primary, policy={}",
                o.policy
            );
        }
    }

    #[test]
    fn low_br_triggers_otp_with_dv() {
        let o = decide_product_action(&DecisionInput {
            bot_verdict: "human".into(),
            device_id: Some("dv_0123456789abcdef".into()),
            soft_edge: false,
            soft_promote: false,
            confidence: 0.9,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::NonSensitive,
            os_score: Some(0.9),
            br_score: Some(0.2),
            rpa_score: Some(0.9),
        });
        assert!(o.require_otp);
        assert!(o.reasons.iter().any(|r| r.contains("br_score_lt_policy")));
    }
}
