//! T0 capabilities bitmap (G-ARCH-21). Control metadata for brain filtering — not digest material.

use serde_json::{json, Value};

/// Project a compact capabilities object from collected fields / FE probes.
/// Missing keys default to unknown (null) rather than false — avoids false negatives.
pub fn project_capabilities(fields: &Value) -> Value {
    let fo = fields.as_object();
    let get_bool = |k: &str| -> Option<bool> {
        fo.and_then(|o| o.get(k)).and_then(|v| {
            if let Some(b) = v.as_bool() {
                Some(b)
            } else if v.is_null() {
                None
            } else {
                Some(true)
            }
        })
    };
    let has = |k: &str| -> bool {
        fo.and_then(|o| o.get(k))
            .map(|v| !v.is_null() && v.as_str() != Some(""))
            .unwrap_or(false)
    };

    let webgl = get_bool("webgl_supported")
        .or_else(|| {
            if has("webgl_renderer") || has("webgl_vendor") || has("hw_curve_webgl") {
                Some(true)
            } else {
                None
            }
        });
    let workers = get_bool("worker_ok")
        .or_else(|| get_bool("consistency_probe"))
        .or_else(|| {
            fo.and_then(|o| o.get("sandbox"))
                .and_then(|_| Some(true))
        });
    let storage = get_bool("local_storage")
        .or_else(|| get_bool("session_storage"))
        .or_else(|| {
            if has("storage_estimate") || has("indexed_db") {
                Some(true)
            } else {
                None
            }
        });
    let audio = get_bool("audio_ok").or_else(|| {
        if has("hw_curve_audio") || has("audio_fingerprint") {
            Some(true)
        } else {
            None
        }
    });
    let canvas = if has("hw_curve_canvas") || has("canvas_hash") {
        Some(true)
    } else {
        None
    };

    json!({
        "tier": "T0",
        "webgl": webgl,
        "workers": workers,
        "storage": storage,
        "audio": audio,
        "canvas": canvas,
        "webdriver": get_bool("webdriver"),
        "hardware_concurrency": fo.and_then(|o| o.get("hardware_concurrency")).cloned(),
        "device_memory": fo.and_then(|o| o.get("device_memory")).cloned(),
    })
}

/// Drop packs that hard-require a capability known to be unavailable.
pub fn filter_packs_by_capabilities(packs: &[Value], caps: &Value) -> Vec<Value> {
    packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            // Only filter when capability is explicitly false (unknown = keep).
            if pid == "B10_hw_curves" || pid == "B2_hardware" {
                if caps.get("webgl") == Some(&json!(false))
                    && caps.get("canvas") == Some(&json!(false))
                    && caps.get("audio") == Some(&json!(false))
                {
                    return false;
                }
            }
            if pid == "B7_sandbox" {
                if caps.get("workers") == Some(&json!(false)) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_t0_tier() {
        let caps = project_capabilities(&json!({
            "webgl_renderer": "ANGLE",
            "hw_curve_audio": [1, 2, 3],
            "webdriver": false
        }));
        assert_eq!(caps["tier"], "T0");
        assert_eq!(caps["webgl"], true);
        assert_eq!(caps["audio"], true);
        assert_eq!(caps["webdriver"], false);
    }

    #[test]
    fn filters_sandbox_when_workers_false() {
        let packs = vec![
            json!({"pack_id": "B7_sandbox", "priority": 1}),
            json!({"pack_id": "B0_bootstrap", "priority": 2}),
        ];
        let caps = json!({"workers": false});
        let out = filter_packs_by_capabilities(&packs, &caps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["pack_id"], "B0_bootstrap");
    }
}
