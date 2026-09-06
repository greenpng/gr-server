//! Dual-name configuration reads for the GR naming migration (docs/13).
//!
//! Rule (charter §4): read with `GR_*` preference, fall back to `GR_*` then
//! `GR_*`; new code writes `GR_*` only. Keeps 8.0 deployments readable by
//! billing-cycle-old nodes (which still set `GR_*`/`GR_*` names) while new
//! installs are generated with `GR_*` exclusively.

/// Read a config value with `GR_` first, then `GR_`, then `GR_`.
///
/// ```
/// let _ = gr_abi::env::get("DEPLOY_ENV");
/// ```
pub fn get(name: &str) -> Option<String> {
    ["GR_", "GR_", "GR_"]
        .iter()
        .map(|p| format!("{p}{name}"))
        .find_map(|k| std::env::var(&k).ok())
}

/// Same lookup, returning `default` when unset.
pub fn get_or(name: &str, default: &str) -> String {
    get(name).unwrap_or_else(|| default.to_string())
}

/// Same lookup for "1"/"true"/"yes"/"on"-style flags.
pub fn flag(name: &str) -> bool {
    get(name)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

/// The exact env var name that won the lookup, useful for diagnostics/logging
/// (e.g. "read GR_DEPLOY_ENV"). Returns `GR_`-prefixed name when unset.
pub fn resolved_name(name: &str) -> String {
    for p in ["GR_", "GR_", "GR_"] {
        let k = format!("{p}{name}");
        if std::env::var(&k).is_ok() {
            return k;
        }
    }
    format!("GR_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gr_preferred_over_legacy() {
        unsafe {
            std::env::remove_var("GR_TEST_VAR");
            std::env::remove_var("GR_TEST_VAR");
            std::env::remove_var("GR_TEST_VAR");
        }
        unsafe {
            std::env::set_var("GR_TEST_VAR", "six");
            std::env::set_var("GR_TEST_VAR", "five");
        }
        assert_eq!(get("TEST_VAR").as_deref(), Some("six"), "GR fallback");
        assert_eq!(resolved_name("TEST_VAR"), "GR_TEST_VAR");
        unsafe {
            std::env::set_var("GR_TEST_VAR", "gr");
        }
        assert_eq!(get("TEST_VAR").as_deref(), Some("gr"), "GR wins");
        assert_eq!(resolved_name("TEST_VAR"), "GR_TEST_VAR");
        unsafe {
            std::env::remove_var("GR_TEST_VAR");
            std::env::remove_var("GR_TEST_VAR");
        }
        assert_eq!(get("TEST_VAR").as_deref(), Some("five"), "GR fallback");
        unsafe {
            std::env::remove_var("GR_TEST_VAR");
        }
        assert_eq!(get("TEST_VAR"), None);
        assert_eq!(get_or("TEST_VAR", "d"), "d");
    }
}
