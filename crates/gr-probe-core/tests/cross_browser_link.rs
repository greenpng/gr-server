//! Cross-browser / multi-session link + device projection.
//! Table-driven dual-path matrix: associate AND evaluate_session on every row.
//! No engine-name oracles; synthetic multi-source field deltas only.

use gr_probe_core::{
    associate, commercial_device_id_from_fields, evaluate_session, gpu_key, multi_session_link,
    normalize_webgl_renderer,
};
use serde_json::{json, Value};

fn host_curves() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    // Identical live curves → same commercial anchors across browsers/IPs.
    let audio: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.07).sin() * 0.5 + 0.1)
        .collect();
    let canvas: Vec<f64> = (0..16).map(|i| (i as f64 + 1.0) / 32.0).collect();
    let webgl: Vec<f64> = (0..16).map(|i| (i * 7 % 256) as f64).collect();
    (audio, canvas, webgl)
}

fn base_fields(renderer: &str) -> Value {
    // Trust-gated commercial id: os + high-trust noise curves (not egress IP / GPU string).
    let (audio, canvas, webgl) = host_curves();
    json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "device_memory": 8,
        "screen_width": 1920,
        "screen_height": 1080,
        "timezone": "Asia/Shanghai",
        "form_class": "desktop",
        "audio_sample_rate": 48000,
        "color_depth": 24,
        "max_touch_points": 0,
        "webgl_unmasked_renderer": renderer,
        "webgl_unmasked_vendor": "Generic",
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    })
}

fn evidence(fields: &Value) -> Value {
    json!({
        "fields": fields,
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    })
}

/// LINK class max: "LINK" allows LINK; "POSSIBLE" allows POSSIBLE or UNRELATED but not LINK; "UNRELATED" only UNRELATED.
fn decision_ok(got: &str, max_class: &str) -> bool {
    match max_class {
        "LINK" => got == "LINK" || got == "POSSIBLE" || got == "UNRELATED",
        "POSSIBLE" => got == "POSSIBLE" || got == "UNRELATED",
        "UNRELATED" => got == "UNRELATED",
        _ => false,
    }
}

/// Strict: expect LINK class exactly when expect_link_exact is Some.
struct MatrixRow {
    name: &'static str,
    renderer_a: &'static str,
    renderer_b: &'static str,
    /// Optional field overrides on side B only (e.g. timezone, screen).
    delta_b: Option<Value>,
    expect_same_gpu_key: bool,
    /// Maximum link class allowed ("LINK" means may be LINK; "POSSIBLE" must not be LINK).
    expect_link_max: &'static str,
    /// When true, require associate decision == LINK.
    require_link: bool,
    expect_same_device_id: bool,
}

fn matrix() -> Vec<MatrixRow> {
    vec![
        MatrixRow {
            name: "bare_amd_vs_angle_d3d11",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0, D3D11)",
            delta_b: None,
            expect_same_gpu_key: true,
            expect_link_max: "LINK",
            require_link: true,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "bare_amd_vs_google_vulkan_amd",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "ANGLE (Google, Vulkan 1.3.0 (AMD Radeon RX 580 Series (0x67DF)))",
            delta_b: None,
            expect_same_gpu_key: true,
            expect_link_max: "LINK",
            require_link: true,
            expect_same_device_id: true,
        },
        // Machine-stable device_id ignores GPU; hard veto only for two real disagreeing GPUs.
        MatrixRow {
            name: "gtx1660_vs_rtx3060_real_gpus",
            renderer_a: "ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 Direct3D11 vs_5_0 ps_5_0)",
            renderer_b: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0)",
            delta_b: None,
            expect_same_gpu_key: false,
            expect_link_max: "UNRELATED",
            require_link: false,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "rx580_vs_rx6800",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "AMD Radeon RX 6800 XT",
            delta_b: None,
            expect_same_gpu_key: false,
            expect_link_max: "UNRELATED",
            require_link: false,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "rx580_vs_rx570_model_digits",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "AMD Radeon RX 570 Series",
            delta_b: None,
            expect_same_gpu_key: false,
            expect_link_max: "UNRELATED",
            require_link: false,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "uhd630_vs_uhd620_model_digits",
            renderer_a: "Intel(R) UHD Graphics 630",
            renderer_b: "Intel(R) UHD Graphics 620",
            delta_b: None,
            expect_same_gpu_key: false,
            expect_link_max: "UNRELATED",
            require_link: false,
            expect_same_device_id: true,
        },
        // Soft-GL vs real GPU: associate must not LINK/MACHINE_BOUND (max POSSIBLE).
        // Residual-led multi-segment commercial body may still agree when live curves match
        // (GPU string is label-only and excluded from residual mint body).
        MatrixRow {
            name: "swiftshader_vs_real_amd_same_machine",
            renderer_a: "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))",
            renderer_b: "AMD Radeon RX 580 Series",
            delta_b: None,
            expect_same_gpu_key: false,
            expect_link_max: "POSSIBLE",
            require_link: false,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "same_gpu_timezone_spoof",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "AMD Radeon RX 580 Series",
            delta_b: Some(json!({"timezone": "UTC"})),
            expect_same_gpu_key: true,
            // Timezone is low-trust (context spoofable); commercial id stays same via hardware anchors.
            expect_link_max: "LINK",
            require_link: false,
            expect_same_device_id: true,
        },
        MatrixRow {
            name: "same_gpu_viewport_spoof",
            renderer_a: "AMD Radeon RX 580 Series",
            renderer_b: "AMD Radeon RX 580 Series",
            delta_b: Some(json!({"screen_width": 800, "screen_height": 600})),
            expect_same_gpu_key: true,
            expect_link_max: "LINK",
            require_link: true,
            expect_same_device_id: true,
        },
    ]
}

#[test]
fn dual_path_gpu_link_device_id_matrix() {
    for row in matrix() {
        let fields_a = base_fields(row.renderer_a);
        let mut fields_b = base_fields(row.renderer_b);
        if let Some(delta) = &row.delta_b {
            if let (Some(obj), Some(d)) = (fields_b.as_object_mut(), delta.as_object()) {
                for (k, v) in d {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        let ka = gpu_key(row.renderer_a);
        let kb = gpu_key(
            fields_b
                .get("webgl_unmasked_renderer")
                .and_then(|v| v.as_str())
                .unwrap_or(row.renderer_b),
        );
        assert_eq!(
            ka == kb,
            row.expect_same_gpu_key,
            "[{}] gpu_key same? a={ka:?} b={kb:?}",
            row.name
        );

        let link = associate(&fields_a, &fields_b, "sparse_safe").unwrap();
        assert!(
            decision_ok(&link.decision, row.expect_link_max),
            "[{}] link decision {} exceeds max {} vetoes={:?} details={:?}",
            row.name,
            link.decision,
            row.expect_link_max,
            link.vetoes,
            link.details
        );
        if row.require_link {
            assert_eq!(
                link.decision, "LINK",
                "[{}] expected LINK got {} {:?}",
                row.name, link.decision, link
            );
        }
        // Closed invariant: LINK ⇒ equal commercial device_ids
        if link.decision == "LINK" {
            let ida = commercial_device_id_from_fields(&fields_a);
            let idb = commercial_device_id_from_fields(&fields_b);
            assert_eq!(
                ida, idb,
                "[{}] LINK without same device_id: {:?} vs {:?}",
                row.name, ida, idb
            );
        }

        let out_a = evaluate_session(&evidence(&fields_a), None, Some(&fields_b), None, true).unwrap();
        let out_b = evaluate_session(&evidence(&fields_b), None, Some(&fields_a), None, true).unwrap();
        let id_a = out_a
            .pointer("/device/device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let id_b = out_b
            .pointer("/device/device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            !id_a.is_empty() && id_a == id_b,
            row.expect_same_device_id,
            "[{}] device_id same? a={id_a} b={id_b} link={}",
            row.name,
            link.decision
        );
        if row.expect_same_device_id {
            assert!(!id_a.is_empty(), "[{}] empty device_id", row.name);
        }
        // evaluate-path link also must not violate LINK⇒same id
        if let Some(dec) = out_a
            .get("link")
            .and_then(|l| l.get("decision"))
            .and_then(|d| d.as_str())
        {
            if dec == "LINK" {
                assert_eq!(
                    id_a, id_b,
                    "[{}] evaluate link=LINK but device_ids diverge",
                    row.name
                );
            }
        }
    }
}

/// Chromium-only soft fields + different proxy IPs must not split commercial device_id
/// when high-trust hardware noise curves match on the same host.
#[test]
fn chrome_extra_soft_fields_same_device_id_as_firefox() {
    let (audio, canvas, webgl) = host_curves();
    let chrome = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "device_memory": 8,
        "timezone": "Asia/Shanghai",
        "audio_sample_rate": 48000,
        "color_depth": 24,
        "max_touch_points": 0,
        "server_client_ip": "203.0.113.10",
        "webgl_unmasked_renderer": "ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0)",
        "form_class": "desktop",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    });
    let (audio, canvas, webgl) = host_curves();
    let firefox = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "audio_sample_rate": 48000,
        "color_depth": 24,
        "max_touch_points": 0,
        "server_client_ip": "198.51.100.20",
        // Real hard path (not soft-GL): commercial id associates via curves, not soft labels.
        "webgl_unmasked_renderer": "AMD Radeon RX 580 Series",
        "form_class": "desktop",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    });
    let (audio, canvas, webgl) = host_curves();
    let webkit = json!({
        "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.5 Safari/605.1.15",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "Europe/London",
        "color_depth": 24,
        "max_touch_points": 0,
        "server_client_ip": "198.51.100.99",
        "webgl_unmasked_renderer": "AMD Radeon RX 580 Series",
        "form_class": "desktop",
        "screen_width": 1280,
        "screen_height": 720,
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    });
    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome id");
    let id_f = commercial_device_id_from_fields(&firefox).expect("firefox id");
    let id_w = commercial_device_id_from_fields(&webkit).expect("webkit id");
    assert_eq!(id_c, id_f, "chrome vs firefox same commercial id across proxy IPs");
    assert_eq!(id_c, id_w, "chrome vs webkit same commercial id (ignore cores/UA Mac lie/IP)");
    let link = associate(&chrome, &firefox, "sparse_safe").unwrap();
    assert_eq!(
        link.decision, "LINK",
        "same-machine chrome/firefox should LINK: {:?}",
        link
    );
    let link_w = associate(&chrome, &webkit, "sparse_safe").unwrap();
    assert_eq!(
        link_w.decision, "LINK",
        "same-machine chrome/webkit should LINK: {:?}",
        link_w
    );
}

/// Platform-first OS: WebKit Macintosh UA + Linux platform → linux; commercial id needs anchors.
#[test]
fn webkit_mac_ua_linux_platform_same_os_family() {
    let (audio, canvas, webgl) = host_curves();
    // Same dual silicon + LAN host sep; cores/IP/UA surface may differ.
    let linux = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "architecture": "x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "server_client_ip": "203.0.113.10",
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    });
    let (audio, canvas, webgl) = host_curves();
    let webkit_edge = json!({
        "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "architecture": "x86_64",
        "hardware_concurrency": 8,
        "timezone": "Asia/Shanghai",
        "server_client_ip": "198.51.100.1",
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "hw_curve_webgl": webgl,
        "webrtc_host_ip_hash": "h_same_lan_aabb",
    });
    let id_l = commercial_device_id_from_fields(&linux).expect("linux");
    let id_w = commercial_device_id_from_fields(&webkit_edge).expect("webkit");
    assert_eq!(id_l, id_w, "trust-gated commercial id ignores cores and proxy IP");
    assert!((id_l.starts_with("dv-") || id_l.starts_with("dv_")), "{id_l}");
}

#[test]
fn gpu_key_not_collapse_google_vulkan() {
    let amd = gpu_key("ANGLE (Google, Vulkan 1.3.0 (AMD Radeon RX 580 Series))");
    let nv = gpu_key("ANGLE (Google, Vulkan 1.3.0 (NVIDIA GeForce RTX 3060))");
    assert!(amd.contains("radeon") || amd.contains("580") || amd.contains("amd"), "{amd}");
    assert_ne!(amd, nv);
    assert_ne!(amd, "google");
}

#[test]
fn model_digits_preserved_in_gpu_key() {
    let a = gpu_key("Intel(R) UHD Graphics 630");
    let b = gpu_key("Intel(R) UHD Graphics 620");
    assert!(a.contains("630"), "{a}");
    assert!(b.contains("620"), "{b}");
    assert_ne!(a, b);
    let c = gpu_key("AMD Radeon RX 580 Series");
    let d = gpu_key("AMD Radeon RX 570 Series");
    assert!(c.contains("580"), "{c}");
    assert!(d.contains("570"), "{d}");
    assert_ne!(c, d);
}

#[test]
fn thin_session_never_confirmed_real() {
    let evidence = json!({
        "fields": { "user_agent": "Mozilla/5.0" },
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}]
    });
    let out = evaluate_session(&evidence, None, None, None, true).unwrap();
    let band = out.get("real_band").and_then(|v| v.as_str()).unwrap_or("");
    assert_ne!(band, "confirmed_real");
}

#[test]
fn automation_webdriver_not_soft_promoted() {
    let evidence = json!({
        "fields": {
            "webdriver": true,
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/120.0.0.0",
            "platform": "Linux x86_64",
            "hardware_concurrency": 4,
            "screen_width": 800,
            "screen_height": 600,
            "timezone": "UTC",
            "webgl_unmasked_renderer": "Google SwiftShader",
            "automation": {"webdriver": true, "playwright": true}
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let out = evaluate_session(&evidence, None, None, None, true).unwrap();
    assert_eq!(
        out.pointer("/device/soft_promote").and_then(|v| v.as_bool()),
        Some(false)
    );
    let band = out.get("real_band").and_then(|v| v.as_str()).unwrap_or("");
    assert_ne!(band, "confirmed_real", "band={band}");
}

#[test]
fn multi_session_link_api() {
    let a = base_fields("AMD Radeon RX 580 Series");
    let b = base_fields("ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0, D3D11)");
    let v = multi_session_link(&a, &b, "sparse_safe").unwrap();
    let decision = v.get("decision").and_then(|x| x.as_str()).unwrap_or("");
    assert_eq!(decision, "LINK", "{v}");
}

#[test]
fn normalize_still_strips_wrappers() {
    let n = normalize_webgl_renderer(
        "ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0, D3D11)",
    );
    assert!(!n.contains("angle"), "{n}");
    assert!(n.contains("radeon") || n.contains("580"), "{n}");
}

#[test]
fn desktop_vs_mobile_still_vetoed() {
    let desk = json!({
        "os_family": "android",
        "hardware_concurrency": 8,
        "screen_width": 1920,
        "screen_height": 1080,
        "timezone": "UTC",
        "form_class": "desktop",
        "webgl_unmasked_renderer": "Mali-G78"
    });
    let mob = json!({
        "os_family": "android",
        "hardware_concurrency": 8,
        "screen_width": 390,
        "screen_height": 844,
        "timezone": "UTC",
        "form_class": "mobile",
        "webgl_unmasked_renderer": "Mali-G78"
    });
    let r = associate(&desk, &mob, "sparse_safe").unwrap();
    assert_eq!(r.decision, "UNRELATED");
    assert!(r.vetoes.iter().any(|v| v == "form_class_desktop_vs_mobile"));
}
