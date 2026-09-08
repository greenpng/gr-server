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
    /// iss/ota-unattended-auto-upgrade-design L1: unattended auto-apply switch.
    /// Rows written before the field existed deserialize to `false` — declaring
    /// a desired release alone keeps the panel-driven (manual) semantics.
    #[serde(default)]
    pub auto_apply: bool,
    /// L1: cold-part maintenance window, `"HH:MM-HH:MM"` (cross-midnight ok,
    /// comma-separated list ok). Empty = unrestricted. Hot parts (modules/FE)
    /// ignore the window; the root auto-upgrade timer honors it.
    #[serde(default)]
    pub window: String,
    #[serde(default)]
    pub written_ms: i64,
}

/// Hot auto-OTA mode from `GR_AUTO_OTA_HOT` (gr_abi reads `GR_`-prefixed).
/// `1`/`enforce`/`on`/`true` → apply unattended; unset/`warn`/`0`/`off` →
/// log the would-apply decision and touch nothing (observation default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotOtaMode {
    Enforce,
    Warn,
}

pub fn hot_ota_mode(raw: &str) -> HotOtaMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "enforce" | "on" | "true" | "yes" => HotOtaMode::Enforce,
        _ => HotOtaMode::Warn,
    }
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// Strict semver `a > b` for `X.Y.Z`. Unparseable input → false (fail closed:
/// callers use this as an upgrade-only gate, so "unknown" must not authorize).
pub fn version_gt(a: &str, b: &str) -> bool {
    fn parse(s: &str) -> Option<(u64, u64, u64)> {
        let mut it = s.trim().split('.');
        let maj = it.next()?.parse().ok()?;
        let min = it.next()?.parse().ok()?;
        let pat = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some((maj, min, pat))
    }
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

/// A window is valid when it is empty (unrestricted) or has at least one
/// parseable `HH:MM-HH:MM` range. Used as a write-time guard so bad input is
/// rejected at the declaring endpoint, not discovered by the timer at 04:30.
pub fn window_valid(window: &str) -> bool {
    let w = window.trim();
    if w.is_empty() {
        return true;
    }
    for part in w.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            if parse_hhmm(a).is_some() && parse_hhmm(b).is_some() {
                return true;
            }
        }
    }
    false
}

/// True when `hhmm` (`"HH:MM"`, local time) falls inside `window`.
/// Grammar: comma-separated `HH:MM-HH:MM` ranges, end-exclusive, cross-midnight
/// wraps (`"22:00-06:00"`). Empty window → true (unrestricted). A window with no
/// parseable matching range → false — the cold-part gate fails closed on
/// unparseable input instead of running outside the operator's window.
pub fn window_contains(window: &str, hhmm: &str) -> bool {
    let w = window.trim();
    if w.is_empty() {
        return true;
    }
    let Some(now) = parse_hhmm(hhmm) else {
        return false;
    };
    for part in w.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((a, b)) = part.split_once('-') else {
            continue;
        };
        let (Some(start), Some(end)) = (parse_hhmm(a), parse_hhmm(b)) else {
            continue;
        };
        if start <= end {
            if now >= start && now < end {
                return true;
            }
        } else if now >= start || now < end {
            // cross-midnight: "22:00-06:00" == [22:00..24:00) ∪ [00:00..06:00)
            return true;
        }
    }
    false
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

/// Unattended hot-apply decision from the active module versions vs desired.
///
/// Completion is "every active module sits at the desired version" — an
/// analyze-only completion signal closes the gate while other modules still
/// lag (178 v1.0.6: analyze landed first over a flaky link, the gate closed
/// and the remaining four modules stalled on 1.0.5). Upgrade-only, mirroring
/// `ota_activate`'s floor and the L3 timer's monotonic gate: a desired below
/// any active version is blocked; a desired equal to the max active with
/// laggards is a catch-up apply, not a downgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotApplyDecision {
    /// Desired differs from the active set and is not below any active version.
    Apply,
    /// Every active module already sits at the desired version.
    Closed,
    /// Desired is below at least one active version — never downgrade unattended.
    DowngradeBlocked,
}

/// `versions` = versions of the node's active modules (marker set; see
/// `OtaEngine::active_modules`); empty (fresh install, no markers) bootstraps
/// with an apply.
pub fn hot_apply_decision(versions: &[&str], desired: &str) -> HotApplyDecision {
    let desired = desired.trim();
    if versions.is_empty() {
        return HotApplyDecision::Apply;
    }
    if !versions.iter().any(|v| v.trim() != desired) {
        return HotApplyDecision::Closed;
    }
    if versions
        .iter()
        .any(|v| version_gt(v.trim(), desired))
    {
        return HotApplyDecision::DowngradeBlocked;
    }
    HotApplyDecision::Apply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desired(url: &str, version: &str) -> ClusterOtaDesired {
        ClusterOtaDesired {
            release_url: url.into(),
            version: version.into(),
            activate: true,
            auto_apply: false,
            window: String::new(),
            written_ms: 1,
        }
    }

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
        let d = desired(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.24",
            "6.0.24",
        );
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
        let d = desired("", "");
        assert!(!should_apply_cluster_ota("https://x/v1", Some("1"), &d));
    }

    #[test]
    fn hot_apply_decision_gates() {
        use HotApplyDecision::*;
        // nothing loaded → bootstrap apply
        assert_eq!(hot_apply_decision(&[], "1.0.6"), Apply);
        // all at desired → closed
        assert_eq!(
            hot_apply_decision(&["1.0.6", "1.0.6", "1.0.6"], "1.0.6"),
            Closed
        );
        // uniform upgrade
        assert_eq!(
            hot_apply_decision(&["1.0.5", "1.0.5", "1.0.5"], "1.0.6"),
            Apply
        );
        // 178 v1.0.6 regression: analyze+ingest at desired, four lagging — the
        // gate must stay open as a catch-up apply, not close on analyze alone
        assert_eq!(
            hot_apply_decision(&["1.0.6", "1.0.6", "1.0.5", "1.0.5", "1.0.5", "1.0.5"], "1.0.6"),
            Apply
        );
        // desired below one active module → downgrade blocked (floor parity)
        assert_eq!(
            hot_apply_decision(&["1.0.6", "1.0.5"], "1.0.5"),
            DowngradeBlocked
        );
        assert_eq!(
            hot_apply_decision(&["1.0.6", "1.0.5"], "1.0.4"),
            DowngradeBlocked
        );
        // unparseable active version does not authorize a downgrade (version_gt
        // fails closed) but also does not block a legitimate catch-up
        assert_eq!(
            hot_apply_decision(&["1.0.6", "dev-local"], "1.0.6"),
            Apply
        );
    }

    #[test]
    fn hot_mode_env_values() {
        assert_eq!(hot_ota_mode("1"), HotOtaMode::Enforce);
        assert_eq!(hot_ota_mode("enforce"), HotOtaMode::Enforce);
        assert_eq!(hot_ota_mode("ON"), HotOtaMode::Enforce);
        assert_eq!(hot_ota_mode("true"), HotOtaMode::Enforce);
        assert_eq!(hot_ota_mode(""), HotOtaMode::Warn);
        assert_eq!(hot_ota_mode("warn"), HotOtaMode::Warn);
        assert_eq!(hot_ota_mode("0"), HotOtaMode::Warn);
        assert_eq!(hot_ota_mode("off"), HotOtaMode::Warn);
    }

    #[test]
    fn window_membership() {
        // empty = unrestricted
        assert!(window_contains("", "23:59"));
        assert!(window_contains("   ", "03:00"));
        // normal range, end-exclusive
        assert!(window_contains("04:00-05:00", "04:00"));
        assert!(window_contains("04:00-05:00", "04:59"));
        assert!(!window_contains("04:00-05:00", "05:00"));
        assert!(!window_contains("04:00-05:00", "03:59"));
        // cross-midnight wraps
        assert!(window_contains("22:00-06:00", "23:30"));
        assert!(window_contains("22:00-06:00", "00:00"));
        assert!(window_contains("22:00-06:00", "05:59"));
        assert!(!window_contains("22:00-06:00", "06:00"));
        assert!(!window_contains("22:00-06:00", "12:00"));
        // comma list; malformed parts are skipped, all-bad fails closed
        assert!(window_contains("bad,04:00-05:00", "04:30"));
        assert!(!window_contains("bad,04:00-05:00", "06:30"));
        assert!(!window_contains("not-a-window", "04:30"));
        // malformed "now" fails closed
        assert!(!window_contains("04:00-05:00", "25:00"));
        assert!(!window_contains("04:00-05:00", "garbage"));
        // validity guard: empty ok, one good range ok, all-bad rejected
        assert!(window_valid(""));
        assert!(window_valid("04:00-05:00"));
        assert!(window_valid("bad,04:00-05:00"));
        assert!(!window_valid("not-a-window"));
        assert!(!window_valid("04:00"));
        assert!(!window_valid("04:00-25:00"));
    }

    #[test]
    fn semver_gt_fail_closed() {
        assert!(version_gt("1.0.6", "1.0.5"));
        assert!(version_gt("1.1.0", "1.0.99"));
        assert!(version_gt("2.0.0", "1.9.9"));
        assert!(!version_gt("1.0.5", "1.0.5"));
        assert!(!version_gt("1.0.4", "1.0.5"));
        assert!(!version_gt("1.0.5", "1.0.5-lab")); // unparseable → fail closed
        assert!(!version_gt("", "1.0.5"));
        assert!(!version_gt("1.0", "1.0.5"));
    }

    #[test]
    fn old_desired_rows_deserialize_with_defaults() {
        // Rows written before iss/ota-auto fields existed keep manual semantics.
        let old = r#"{"release_url":"https://x/download/v1.0.5","version":"1.0.5","activate":true,"written_ms":7}"#;
        let d: ClusterOtaDesired = serde_json::from_str(old).expect("old row must parse");
        assert!(!d.auto_apply);
        assert_eq!(d.window, "");
        // New round-trip keeps the fields.
        let n = ClusterOtaDesired {
            auto_apply: true,
            window: "04:00-05:00".into(),
            ..desired("https://x/download/v1.0.6", "1.0.6")
        };
        let back: ClusterOtaDesired =
            serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
    }
}
