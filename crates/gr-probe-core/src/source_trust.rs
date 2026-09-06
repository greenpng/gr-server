//! Source authenticity ladder for multi-source field resolution.
//!
//! **Product rule (2026-08)**: on conflict, do **not** blindly prefer `main`.
//! Prefer the source that is **harder to forge** for that key class.
//!
//! ## Threat model (summary)
//!
//! | Source | Who can forge | Strength |
//! |--------|---------------|----------|
//! | **gateway / TLS JA4·H2** | Custom TLS stack / non-browser clients | Edge-observed; page JS **cannot** write |
//! | **cloudflare headers** | Spoof only if not behind real CF; CF-IP/bot from CF edge | Network layer; not JS |
//! | **worker nest** | Isolated realm; main-page `navigator` overrides often **don't** apply | Good for soft identity |
//! | **iframe nest** | Same-origin nest can still be owned if attacker owns page | Consistency check |
//! | **main FE** | Full attacker control of labels + many APIs | Highest fidelity silicon; easiest soft spoof |
//!
//! ## Key classes
//!
//! - **protocol**: engine from JA4/H2 — gateway authoritative vs FE UA
//! - **network**: IP/ASN/geo — gateway/CF authoritative
//! - **soft_identity**: platform/cores/webdriver/tz — prefer worker > iframe > main
//! - **silicon**: residual/curves — main high fidelity; nest validates; hard conflict → no mint
//! - **viewport**: screen size — main preferred (nest iframe is 2×2)

use serde_json::{json, Map, Value};

/// Higher = harder for page JS to forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SourceTrust {
    /// Unknown / merged bag without provenance
    Unknown = 0,
    /// Main page JS soft labels (UA string, spoofable navigator)
    MainSoft = 30,
    /// Main page silicon APIs (WebGL/audio) — high fidelity but JS-callable
    MainSilicon = 45,
    /// Same-origin iframe nest identity
    IframeNest = 55,
    /// Dedicated worker nest (isolated globals)
    WorkerNest = 70,
    /// Cloudflare edge headers (when traffic truly via CF)
    Cloudflare = 85,
    /// Our gateway / reverse-proxy observed TLS·H2·client IP
    GatewayEdge = 95,
}

impl SourceTrust {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::MainSoft => "main_soft",
            Self::MainSilicon => "main_silicon",
            Self::IframeNest => "iframe_nest",
            Self::WorkerNest => "worker_nest",
            Self::Cloudflare => "cloudflare",
            Self::GatewayEdge => "gateway_edge",
        }
    }
}

/// Semantic class of a field for trust ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClass {
    /// TLS/JA4/H2/protocol_engine
    Protocol,
    /// server_client_ip, ASN, country
    Network,
    /// platform, os_family, cores, webdriver, timezone, language
    SoftIdentity,
    /// residual, hw_curve_*, unit_surface
    Silicon,
    /// screen_width/height, form_class (viewport-sensitive)
    Viewport,
    /// media device counts, display_count
    Inventory,
    Other,
}

pub fn key_class(key: &str) -> KeyClass {
    match key {
        "ja4" | "tls_ja4" | "ja3" | "h2_fingerprint" | "http2_fingerprint" | "protocol_engine"
        | "tls_version" | "tls_alpn" | "ja4_r" => KeyClass::Protocol,
        "server_client_ip" | "server_asn" | "server_country" | "cf_connecting_ip"
        | "cf_ipcountry" | "cf_bot_score" | "bot_score" => KeyClass::Network,
        "user_agent" | "gateway_user_agent" | "edge_user_agent" | "platform" | "os_family"
        | "hardware_concurrency" | "device_memory" | "webdriver" | "timezone"
        | "timezone_offset_min" | "language" | "languages" | "vendor" | "architecture"
        | "max_touch_points" | "gateway_accept_language" | "http_header_order_hash" => {
            KeyClass::SoftIdentity
        }
        "residual_std" | "residual_mean" | "residual_ok" | "residual_algo" | "unit_surface_id"
        | "webgl_unmasked_renderer" | "webgl_unmasked_vendor" => KeyClass::Silicon,
        k if k.starts_with("hw_curve_") => KeyClass::Silicon,
        "screen_width" | "screen_height" | "screen_avail_width" | "screen_avail_height"
        | "form_class" | "device_pixel_ratio" | "screen_color_depth" => KeyClass::Viewport,
        "media_input_count" | "media_output_count" | "media_video_count" | "media_device_count"
        | "display_count" | "gl_max_texture_size" | "webgl_max_texture" | "max_texture_size"
        | "claimed_max_tex" | "caps_claimed_max_tex" | "actual_max_tex" => KeyClass::Inventory,
        "webrtc_host_ip_hash" | "webrtc_host_ip_hash_v2" | "os_instance_hash" => {
            // Host seps: edge cannot supply LAN; FE webrtc is best-effort (main/nest).
            KeyClass::Inventory
        }
        _ => KeyClass::Other,
    }
}

/// Map source label → base trust, then adjust by key class.
pub fn source_trust_for(src: &str, key: &str) -> SourceTrust {
    let base = src.split(':').next().unwrap_or(src);
    let class = key_class(key);
    match base {
        "gateway" => SourceTrust::GatewayEdge,
        "cloudflare" | "cf" => SourceTrust::Cloudflare,
        "worker" | "shared_worker" | "service_worker" => SourceTrust::WorkerNest,
        "iframe" | "sandbox" | "sandbox_iframe" => SourceTrust::IframeNest,
        "main" => match class {
            KeyClass::Silicon => SourceTrust::MainSilicon,
            _ => SourceTrust::MainSoft,
        },
        "merged" | "" => SourceTrust::Unknown,
        _ => {
            if base.starts_with("worker") {
                SourceTrust::WorkerNest
            } else if base.starts_with("iframe") {
                SourceTrust::IframeNest
            } else {
                SourceTrust::Unknown
            }
        }
    }
}

/// Whether this source is eligible to supply `key` for mint at all.
pub fn source_can_supply(src: &str, key: &str) -> bool {
    let base = src.split(':').next().unwrap_or(src);
    let class = key_class(key);
    match class {
        KeyClass::Protocol => {
            // Protocol fingerprints only from edge (Pingora / CF), never FE invent
            matches!(base, "gateway" | "cloudflare" | "cf")
        }
        KeyClass::Network => {
            // Network IP: gateway always; CF only when source is cloudflare
            matches!(base, "gateway" | "cloudflare" | "cf")
        }
        KeyClass::Silicon => {
            // Only browser surfaces produce residual/curves
            matches!(
                base,
                "main" | "iframe" | "worker" | "sandbox" | "sandbox_iframe" | "merged"
            )
        }
        KeyClass::Viewport => {
            // Nest 2px iframe viewport is not authoritative for form/screen
            matches!(base, "main" | "merged")
                || (matches!(base, "iframe" | "worker") && key == "form_class")
        }
        KeyClass::SoftIdentity => {
            // gateway_user_agent / edge headers: gateway may supply
            if matches!(
                key,
                "gateway_user_agent"
                    | "edge_user_agent"
                    | "user_agent"
                    | "gateway_accept_language"
                    | "http_header_order_hash"
            ) {
                return matches!(
                    base,
                    "gateway"
                        | "main"
                        | "iframe"
                        | "worker"
                        | "sandbox"
                        | "sandbox_iframe"
                        | "merged"
                        | "cloudflare"
                        | "cf"
                );
            }
            true
        }
        _ => true,
    }
}

/// Rank sources for a key: higher trust first. Stable secondary by name.
pub fn rank_sources_for_key(key: &str, sources: &[(String, Value)]) -> Vec<(String, Value, SourceTrust)> {
    let mut ranked: Vec<(String, Value, SourceTrust)> = sources
        .iter()
        .filter(|(s, _)| source_can_supply(s, key))
        .map(|(s, v)| (s.clone(), v.clone(), source_trust_for(s, key)))
        .collect();
    ranked.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked
}

/// Resolve which source wins for mint value.
///
/// - **Agree** (≥2 approx-equal): pick highest-trust among agreeing set
/// - **Conflict**: pick highest-trust eligible source (harder to forge); flag conflict
/// - **Silicon hard conflict**: do **not** pick a winner for mint (return None) — residual lies are ambiguous
/// - **Single source**: conf-only unless dual-channel handled by caller
#[derive(Debug, Clone)]
pub struct SourceResolution {
    pub chosen_source: Option<String>,
    pub chosen_trust: SourceTrust,
    pub conflicted: bool,
    pub agreed: bool,
    pub mint_value_ok: bool,
    pub reason: &'static str,
}

pub fn resolve_field_source(
    key: &str,
    entries: &[(String, Value)],
    approx_eq: impl Fn(&Value, &Value) -> bool,
) -> SourceResolution {
    let eligible: Vec<(String, Value)> = entries
        .iter()
        .filter(|(s, _)| source_can_supply(s, key))
        .cloned()
        .collect();
    if eligible.is_empty() {
        return SourceResolution {
            chosen_source: None,
            chosen_trust: SourceTrust::Unknown,
            conflicted: false,
            agreed: false,
            mint_value_ok: false,
            reason: "no_eligible_source",
        };
    }
    if eligible.len() == 1 {
        let (s, _) = &eligible[0];
        let t = source_trust_for(s, key);
        return SourceResolution {
            chosen_source: Some(s.clone()),
            chosen_trust: t,
            conflicted: false,
            agreed: false,
            mint_value_ok: false, // single — conf only at field level
            reason: "single_source_conf_only",
        };
    }

    // Cluster by approx equality
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for i in 0..eligible.len() {
        let mut placed = false;
        for c in clusters.iter_mut() {
            if approx_eq(&eligible[c[0]].1, &eligible[i].1) {
                c.push(i);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(vec![i]);
        }
    }

    let class = key_class(key);
    if clusters.len() == 1 {
        // Full agree — class-specific pick among agreeing sources:
        // - Silicon: prefer **main** for measurement fidelity (nest is validation only)
        // - Soft identity: prefer **worker** (harder main-navigator spoof)
        // - Network/protocol: prefer **gateway/CF**
        // - Else: highest trust score
        let ranked = rank_sources_for_key(key, &eligible);
        let pick = match class {
            KeyClass::Silicon => ranked
                .iter()
                .find(|(s, _, _)| s == "main" || s.starts_with("main:"))
                .or_else(|| ranked.first()),
            KeyClass::SoftIdentity => {
                // user_agent / accept: prefer gateway edge when present
                if matches!(
                    key,
                    "user_agent"
                        | "gateway_user_agent"
                        | "edge_user_agent"
                        | "gateway_accept_language"
                        | "http_header_order_hash"
                ) {
                    ranked
                        .iter()
                        .find(|(_, _, t)| {
                            matches!(t, SourceTrust::GatewayEdge | SourceTrust::Cloudflare)
                        })
                        .or_else(|| {
                            ranked.iter().find(|(s, _, t)| {
                                matches!(t, SourceTrust::WorkerNest) || s.starts_with("worker")
                            })
                        })
                        .or_else(|| ranked.first())
                } else {
                    ranked
                        .iter()
                        .find(|(s, _, t)| {
                            matches!(t, SourceTrust::WorkerNest) || s.starts_with("worker")
                        })
                        .or_else(|| ranked.first())
                }
            }
            KeyClass::Network | KeyClass::Protocol => ranked
                .iter()
                .find(|(_, _, t)| {
                    matches!(t, SourceTrust::GatewayEdge | SourceTrust::Cloudflare)
                })
                .or_else(|| ranked.first()),
            _ => ranked.first(),
        };
        let (s, _, t) = pick.expect("ranked non-empty");
        return SourceResolution {
            chosen_source: Some(s.clone()),
            chosen_trust: *t,
            conflicted: false,
            agreed: true,
            mint_value_ok: true,
            reason: "multi_source_agree_class_prefer",
        };
    }

    // Conflict
    if class == KeyClass::Silicon {
        // Ambiguous silicon conflict: never mint residual/curves from a single spoofed surface
        return SourceResolution {
            chosen_source: None,
            chosen_trust: SourceTrust::Unknown,
            conflicted: true,
            agreed: false,
            mint_value_ok: false,
            reason: "silicon_conflict_exclude_mint",
        };
    }

    // Soft / network / protocol: pick harder-to-forge source
    let ranked = rank_sources_for_key(key, &eligible);
    let (s, _, t) = &ranked[0];
    // Prefer nest/worker/gateway over main on soft identity conflict
    let prefer_hard = matches!(
        t,
        SourceTrust::WorkerNest
            | SourceTrust::IframeNest
            | SourceTrust::GatewayEdge
            | SourceTrust::Cloudflare
    ) || (*t >= SourceTrust::MainSilicon);
    SourceResolution {
        chosen_source: Some(s.clone()),
        chosen_trust: *t,
        conflicted: true,
        agreed: false,
        mint_value_ok: prefer_hard, // mint with hard source; still demote scores
        reason: if prefer_hard {
            "conflict_resolved_to_harder_source"
        } else {
            "conflict_weak_source_conf_only"
        },
    }
}

/// Build ops-facing auth view for all mint-relevant keys.
pub fn build_source_auth_ladder_view(
    fields_by_source: &Map<String, Value>,
    keys: &[&str],
    approx_eq: impl Fn(&Value, &Value) -> bool + Copy,
) -> Value {
    let mut map = Map::new();
    for k in keys {
        let mut entries = Vec::new();
        for (src, bucket) in fields_by_source {
            if let Some(obj) = bucket.as_object() {
                if let Some(v) = obj.get(*k) {
                    if !matches!(v, Value::Null) {
                        if let Value::String(s) = v {
                            if s.is_empty() {
                                continue;
                            }
                        }
                        entries.push((src.clone(), v.clone()));
                    }
                }
            }
        }
        let res = resolve_field_source(k, &entries, approx_eq);
        let trusts: Vec<Value> = entries
            .iter()
            .map(|(s, _)| {
                json!({
                    "source": s,
                    "trust": source_trust_for(s, k).as_str(),
                    "score": source_trust_for(s, k) as u8,
                })
            })
            .collect();
        map.insert(
            (*k).into(),
            json!({
                "chosen_source": res.chosen_source,
                "chosen_trust": res.chosen_trust.as_str(),
                "conflicted": res.conflicted,
                "agreed": res.agreed,
                "mint_value_ok": res.mint_value_ok,
                "reason": res.reason,
                "candidates": trusts,
            }),
        );
    }
    json!({
        "algo": "gr_source_trust_ladder_v1",
        "policy": "prefer_harder_to_forge_on_conflict",
        "ladder": [
            "gateway_edge(95)",
            "cloudflare(85)",
            "worker_nest(70)",
            "iframe_nest(55)",
            "main_silicon(45)",
            "main_soft(30)"
        ],
        "keys": map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn eq(a: &Value, b: &Value) -> bool {
        a == b
    }

    #[test]
    fn soft_conflict_prefers_worker_over_main() {
        let entries = vec![
            ("main".into(), json!("Win32")),
            ("worker:d1".into(), json!("Linux x86_64")),
            ("iframe:d1".into(), json!("Linux x86_64")),
        ];
        // platform: worker and iframe agree Linux; main says Win — but resolve picks highest trust
        // Among all, worker has highest trust for soft identity
        let r = resolve_field_source("platform", &entries, eq);
        assert_eq!(r.conflicted, true);
        assert_eq!(r.chosen_source.as_deref(), Some("worker:d1"));
        assert!(r.mint_value_ok);
        assert_eq!(r.reason, "conflict_resolved_to_harder_source");
    }

    #[test]
    fn soft_agree_prefers_worker_even_if_main_present() {
        let entries = vec![
            ("main".into(), json!(12)),
            ("worker:d1".into(), json!(12)),
        ];
        let r = resolve_field_source("hardware_concurrency", &entries, eq);
        assert!(r.agreed);
        assert_eq!(r.chosen_source.as_deref(), Some("worker:d1"));
        assert!(r.mint_value_ok);
        assert_eq!(r.reason, "multi_source_agree_class_prefer");
    }

    #[test]
    fn silicon_agree_prefers_main_fidelity() {
        let entries = vec![
            ("main".into(), json!([0.1, 0.2, 0.3])),
            ("iframe:d1".into(), json!([0.1, 0.2, 0.3])),
        ];
        let r = resolve_field_source("hw_curve_webgl", &entries, eq);
        assert!(r.agreed);
        assert_eq!(r.chosen_source.as_deref(), Some("main"));
    }

    #[test]
    fn silicon_conflict_excludes_mint() {
        let entries = vec![
            ("main".into(), json!([0.1, 0.2, 0.3])),
            ("iframe:d1".into(), json!([0.9, 0.8, 0.7])),
        ];
        let r = resolve_field_source("hw_curve_webgl", &entries, eq);
        assert!(r.conflicted);
        assert!(!r.mint_value_ok);
        assert_eq!(r.reason, "silicon_conflict_exclude_mint");
    }

    #[test]
    fn network_key_only_from_gateway() {
        assert!(!source_can_supply("main", "server_client_ip"));
        assert!(source_can_supply("gateway", "server_client_ip"));
        assert!(source_can_supply("cloudflare", "cf_ipcountry"));
    }

    #[test]
    fn viewport_rejects_iframe_screen() {
        assert!(!source_can_supply("iframe:d1", "screen_width"));
        assert!(source_can_supply("main", "screen_width"));
    }

    #[test]
    fn user_agent_prefers_gateway_over_main_on_agree() {
        let entries2: Vec<(String, Value)> = vec![
            ("main".into(), json!("Mozilla/5.0 Chrome/120")),
            ("gateway".into(), json!("Mozilla/5.0 Chrome/120")),
        ];
        let r = resolve_field_source("user_agent", &entries2, eq);
        assert!(r.agreed);
        assert_eq!(r.chosen_source.as_deref(), Some("gateway"));
    }

    #[test]
    fn user_agent_conflict_picks_gateway() {
        let entries: Vec<(String, Value)> = vec![
            ("main".into(), json!("FakeBot/1.0")),
            ("gateway".into(), json!("Mozilla/5.0 Chrome/120")),
        ];
        let r = resolve_field_source("user_agent", &entries, eq);
        assert!(r.conflicted);
        assert_eq!(r.chosen_source.as_deref(), Some("gateway"));
        assert!(r.mint_value_ok);
    }
}
