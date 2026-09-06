//! Cluster-wide GitHub OTA desired state.
//!
//! Panel `set-release-url` / `full-upgrade` writes a desired release URL + version.
//! Every node (host and lab-multi docker) applies the same GitHub assets — no
//! `docker cp` of a local unsigned tree.

use serde::{Deserialize, Serialize};

pub const CLUSTER_OTA_SETTING_KEY: &str = "cluster_ota_desired";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClusterOtaDesired {
    pub release_url: String,
    pub version: String,
    #[serde(default = "default_true")]
    pub activate: bool,
    #[serde(default)]
    pub written_ms: i64,
}

fn default_true() -> bool {
    true
}

/// Parse `…/download/v6.0.24` or `…/download/6.0.24` → `6.0.24`.
pub fn version_from_release_url(url: &str) -> String {
    let tail = url.trim().trim_end_matches('/').rsplit('/').next().unwrap_or("");
    tail.trim_start_matches('v').to_string()
}

/// Apply when the node is not already on this GitHub release + analyze module.
pub fn should_apply_cluster_ota(
    current_release_url: &str,
    current_analyze_version: Option<&str>,
    desired: &ClusterOtaDesired,
) -> bool {
    if desired.release_url.is_empty() || desired.version.is_empty() {
        return false;
    }
    let cur = current_release_url.trim().trim_end_matches('/');
    let want = desired.release_url.trim().trim_end_matches('/');
    if cur.eq_ignore_ascii_case(want) {
        return match current_analyze_version.map(str::trim).filter(|s| !s.is_empty()) {
            None => true,
            Some(v) => v != desired.version,
        };
    }
    // Same product version via a different fetch base (GitHub vs host ota-mirror).
    // Do not flip the URL — docker nodes cannot reach GitHub and would uninstall
    // a working mirror pointer just because the host wrote the public download URL.
    let cur_ver = version_from_release_url(cur);
    if !cur_ver.is_empty() && cur_ver == desired.version {
        if let Some(v) = current_analyze_version.map(str::trim).filter(|s| !s.is_empty()) {
            if v == desired.version {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_github_download_url() {
        assert_eq!(
            version_from_release_url(
                "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24"
            ),
            "6.0.24"
        );
        assert_eq!(
            version_from_release_url(
                "https://github.com/kullyeilert-jpg/gr-releases/releases/download/6.0.24/"
            ),
            "6.0.24"
        );
    }

    #[test]
    fn apply_when_url_or_analyze_module_differs() {
        let d = ClusterOtaDesired {
            release_url: "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24"
                .into(),
            version: "6.0.24".into(),
            activate: true,
            written_ms: 1,
        };
        assert!(should_apply_cluster_ota("", None, &d));
        assert!(should_apply_cluster_ota(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.1",
            Some("6.0.1"),
            &d
        ));
        assert!(should_apply_cluster_ota(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24",
            None,
            &d
        ));
        assert!(should_apply_cluster_ota(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24",
            Some("6.0.23-encode2"),
            &d
        ));
        assert!(!should_apply_cluster_ota(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24/",
            Some("6.0.24"),
            &d
        ));
        assert!(
            !should_apply_cluster_ota(
                "http://172.29.0.1:28680/v1/ota-mirror/6.0.24",
                Some("6.0.24"),
                &d
            ),
            "same version via ota-mirror must not be overwritten by GitHub URL"
        );
        assert!(should_apply_cluster_ota(
            "http://172.29.0.1:28680/v1/ota-mirror/6.0.23",
            Some("6.0.23"),
            &d
        ));
    }

    #[test]
    fn empty_desired_never_applies() {
        let d = ClusterOtaDesired {
            release_url: String::new(),
            version: String::new(),
            activate: true,
            written_ms: 0,
        };
        assert!(!should_apply_cluster_ota("https://x/v1", Some("1"), &d));
    }
}
