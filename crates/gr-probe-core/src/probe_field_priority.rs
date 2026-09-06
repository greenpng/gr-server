//! Probe field priority for device_id / os / br / rpa (session-local analysis).
//!
//! Product boundary: V5 maximizes probe + produces session results. Cross-vtid
//! association is site-local (SDK consumer), not in this crate.
//!
//! # Silicon / multipath priority (device_id body)
//!
//! 1. Longest complete **raw** timing/residual curves (`hw_curve_*`, multipath residual)
//! 2. Commercial digests (`hw_webgl_stable` / `hw_audio_stable`) when raw absent
//! 3. Never UA, client IP, or GPU model/renderer strings in commercial body
//!
//! # OS segment sources
//!
//! Prefer script-stable: `os_family`, `architecture`, cores class, `timezone`,
//! `os_instance_hash`, `webrtc_host_ip_hash`. Gateway IP/UA assist scoring only.
//!
//! # Gateway protocol (br/rpa observers)
//!
//! `ja4`/`ja4t`, H2 settings+priority, QUIC TP/key_share, TLS depth (key_share/psk/ech).
//! Weak IP-TTL join — never device primary keys.
//!
//! # Batch mapping (priority order for brain maximize)
//!
//! | Priority | Batch | Role |
//! |----------|-------|------|
//! | P0 | B10_hw_curves / B10x_* | Silicon residual curves |
//! | P0 | B2_hardware / B3_system | OS-class + host |
//! | P1 | B19_eme_media (+WebCodecs) | Media capability matrix → os/br |
//! | P1 | B18_webgpu / B46_audio_deep | Secondary silicon |
//! | P1 | B8_gateway | Protocol observers |
//! | P2 | B12 anti / B11 rpa | Fake browser / automation |
//! | P2 | mid/dense/R packs | Maximize schedule floors |

use serde_json::{json, Value};

/// Ops/health projection of field→product role mapping.
pub fn probe_field_priority_json() -> Value {
    json!({
        "algo": "probe_field_priority_v1",
        "cross_vtid_association": false,
        "device_id_materials": {
            "priority": [
                "hw_curve_webgl|webgl_residual_multipath",
                "hw_curve_audio|audio_deep_curve",
                "hw_curve_cpu|cpu_timing_curve",
                "residual_mean/std",
                "os_family/architecture/cores/timezone",
                "os_instance_hash|webrtc_host_ip_hash"
            ],
            "excluded": ["user_agent", "server_client_ip", "webgl_unmasked_renderer", "webgl_governed_renderer"],
            "precision_lanes": ["dv0", "dv4", "dv5", "dv6"],
        },
        "os_br_rpa": {
            "uses_device_silicon": true,
            "webcodec_role": "preference_matrix_conf_only",
            "gateway_protocol_role": "observer_conf_only",
            "demote_on": ["webdriver", "automation", "soft_stack", "fake_os_claim"],
        },
        "batches": {
            "silicon_p0": [
                "B10_hw_curves",
                "B10x_silicon_noderiv",
                "B10x_silicon_rint",
                "B10x_silicon_ulp",
                "B10x_silicon_deep"
            ],
            "os_p0": ["B2_hardware", "B3_system", "B0_bootstrap"],
            "media_p1": ["B19_eme_media", "B18_webgpu", "B46_audio_deep"],
            "infra_p1": ["B47_sab_clock"],
            "gateway_p1": ["B8_gateway", "B8_gateway_early"],
            "anti_p2": ["B12_anti_camouflage", "B11_interaction"],
            "concurrency": {
                "note": "pack_loader resource classes: gpu∥audio∥cpu∥rtc; B10x serial under gpu; B18 after B10 on gpu; B46 audio parallel; B47 sab on cpu parallel",
                "gpu": ["B10_hw_curves", "B10x_*", "B18_webgpu"],
                "audio": ["B46_audio_deep"],
                "cpu": ["B47_sab_clock", "B34_cpu_cache_ladder"]
            }
        },
        // Keep in sync with analyze_schedule defaults (idle 20s / no_result 90s).
        "analyze_triggers": [
            "coverage_complete_100",
            "idle_no_upload",
            "no_result_90s_since_first_upload",
            "pre_cold_demote",
            "explicit_request"
        ],
        "maximize_probe": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_doc_excludes_ua_ip() {
        let j = probe_field_priority_json();
        let excl = j["device_id_materials"]["excluded"].as_array().unwrap();
        assert!(excl.iter().any(|x| x.as_str() == Some("user_agent")));
        assert!(excl.iter().any(|x| x.as_str() == Some("server_client_ip")));
        assert_eq!(j["cross_vtid_association"], false);
        assert_eq!(j["maximize_probe"], true);
    }
}
