//! Compatibility projection for legacy plan metadata.
//!
//! The paid/free product split was retired in 8.0. Every site receives the
//! full visitor-analysis surface. `free` and `paid` remain as wire-compatible
//! labels for old stored policy/token data, but they no longer change behavior.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PLAN_FREE: &str = "free";
pub const PLAN_PAID: &str = "paid";

pub const FULL_DEVICE_PRECISIONS: &[&str] = &["dv0", "dv4", "dv5", "dv6"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEntitlement {
    pub plan: String,
    pub rpa_enabled: bool,
    pub device_precisions: Vec<String>,
    /// Primary public device_id lane for this plan.
    pub primary_device_lane: String,
    /// Multi-node load-balancer feature (docs/guides/08-LB-MODULE.md), available on every install.
    #[serde(default)]
    pub lb_enabled: bool,
    /// Available LB modes ("proxy"/"redirect"/"internal_ip").
    #[serde(default)]
    pub lb_modes: Vec<String>,
}

impl PlanEntitlement {
    pub fn full() -> Self {
        Self {
            // Keep the historical paid label on the wire for compatibility;
            // it is not a capability gate.
            plan: PLAN_PAID.into(),
            rpa_enabled: true,
            device_precisions: FULL_DEVICE_PRECISIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            primary_device_lane: "dv0".into(),
            lb_enabled: true,
            lb_modes: ["proxy", "redirect", "internal_ip"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    pub fn free() -> Self {
        Self::full()
    }

    pub fn paid() -> Self {
        Self::full()
    }

    pub fn from_plan(_plan: &str) -> Self {
        Self::full()
    }

    pub fn allows_precision(&self, lane: &str) -> bool {
        self.device_precisions.iter().any(|p| p == lane)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "plan": self.plan,
            "rpa_enabled": self.rpa_enabled,
            "device_precisions": self.device_precisions,
            "primary_device_lane": self.primary_device_lane,
            "lb_enabled": self.lb_enabled,
            "lb_modes": self.lb_modes,
            "product": "visitor_analysis",
            "intercepts_requests": false,
        })
    }
}

/// Filter `device_id_segments` to the full set of public lanes and select the
/// compatibility fallback lane. A result policy may already have selected a
/// different lane in `identity.device_id`; callers preserve that selection.
pub fn filter_device_identity(segments: &Value, ent: &PlanEntitlement) -> (Option<String>, Value) {
    let Some(obj) = segments.as_object() else {
        return (None, Value::Null);
    };
    let mut out = serde_json::Map::new();
    for lane in &ent.device_precisions {
        if let Some(v) = obj.get(lane) {
            out.insert(lane.clone(), v.clone());
        }
    }
    let primary = out
        .get(&ent.primary_device_lane)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            ent.device_precisions.iter().find_map(|lane| {
                out.get(lane)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty() && *s != "0")
                    .map(|s| s.to_string())
            })
        });
    (primary, Value::Object(out))
}

/// Resolve legacy metadata for panel/status compatibility. The result is
/// always full-featured; signed tokens and old policy files cannot disable
/// product capabilities.
pub fn resolve_site_entitlement(
    site_id: Option<&str>,
    page_host: Option<&str>,
    data_dir: Option<&std::path::Path>,
) -> PlanEntitlement {
    resolve_site_entitlement_status(site_id, page_host, data_dir).0
}

/// Same resolution plus provenance for panel display / audit (06-P0-3:
/// operators must see plan, state, and next-check deadline).
pub fn resolve_site_entitlement_status(
    site_id: Option<&str>,
    page_host: Option<&str>,
    data_dir: Option<&std::path::Path>,
) -> (PlanEntitlement, Value) {
    // Licensing and the official-site vault are compatibility metadata only.
    // They must never change the probe, analysis, device-lane, RPA, or LB
    // surface. Keep the arguments for API compatibility with older callers.
    let _ = (site_id, page_host, data_dir);
    (
        PlanEntitlement::full(),
        json!({
            "entitlement_source": "unified_full",
            "license_state": "not_used_for_capabilities",
            "note": "free and paid labels are compatibility metadata; capabilities are ungated",
        }),
    )
}

/// Apply the compatibility entitlement metadata to a projected
/// `product_public` envelope without removing any product surface.
pub fn apply_to_product_public(mut out: Value, ent: &PlanEntitlement, product: &Value) -> Value {
    let segments = product
        .get("device_id_segments")
        .cloned()
        .or_else(|| out.pointer("/identity/device_id_segments").cloned())
        .unwrap_or(Value::Null);
    let (primary, filtered_segs) = filter_device_identity(&segments, ent);
    let lane = out
        .pointer("/meta/primary_device_lane")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&ent.primary_device_lane)
        .to_string();
    if let Some(id) = out.get_mut("identity").and_then(|v| v.as_object_mut()) {
        if id.get("device_id").map(|v| v.is_null()).unwrap_or(true) {
            if let Some(p) = primary {
                id.insert("device_id".into(), json!(p));
            }
        }
        id.insert("device_id_segments".into(), filtered_segs);
        id.insert(
            "precision_lanes".into(),
            json!(ent.device_precisions.clone()),
        );
        id.insert("primary_device_lane".into(), json!(lane));
    }
    if let Some(obj) = out.as_object_mut() {
        obj.insert("entitlement".into(), ent.to_json());
        obj.insert(
            "product_positioning".into(),
            json!({
                "mode": "request_analysis",
                "intercepts_requests": false,
                "analyzes": ["device_identity", "os", "br", "rpa"],
            }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_policy_is_compatibility_only() {
        let dir = std::env::temp_dir().join(format!(
            "gr-ent-compat-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let policy_path = dir.join("panel_policy.json");
        std::fs::write(
            &policy_path,
            serde_json::to_string(&serde_json::json!({
                "site_entitlements": {
                    "gate-site": {"plan": "paid", "expires_at_ms": 9_999_999_999_999i64}
                }
            }))
            .expect("policy json"),
        )
        .expect("write policy");
        // No license_tokens.json / license_ed25519.pk in dir: no signed path.

        let site = Some("gate-site");
        let (ent, status) = resolve_site_entitlement_status(site, None, Some(&dir));
        assert_eq!(
            status.get("entitlement_source").and_then(|v| v.as_str()),
            Some("unified_full")
        );
        assert!(ent.rpa_enabled);
        assert!(ent.allows_precision("dv0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
