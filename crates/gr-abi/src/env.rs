//! Configuration reads for the greenpng line (GR_* naming).
//!
//! Rule: read and write `GR_*` exclusively. The greenpng 1.0.0 line is a
//! clean break — legacy `GV*_*` alias reads were removed with the 8.x line.

/// Read a config value from `GR_{name}`.
///
/// ```
/// let _ = gr_abi::env::get("DEPLOY_ENV");
/// ```
pub fn get(name: &str) -> Option<String> {
    std::env::var(format!("GR_{name}")).ok()
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
/// (e.g. "read GR_DEPLOY_ENV"). Always `GR_`-prefixed.
pub fn resolved_name(name: &str) -> String {
    format!("GR_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gr_only_reads() {
        unsafe {
            std::env::remove_var("GR_TEST_VAR");
        }
        assert_eq!(get("TEST_VAR"), None);
        assert_eq!(get_or("TEST_VAR", "d"), "d");
        assert_eq!(resolved_name("TEST_VAR"), "GR_TEST_VAR");
        assert!(!flag("TEST_VAR"));
        unsafe {
            std::env::set_var("GR_TEST_VAR", "gr");
        }
        assert_eq!(get("TEST_VAR").as_deref(), Some("gr"));
        assert_eq!(resolved_name("TEST_VAR"), "GR_TEST_VAR");
        unsafe {
            std::env::remove_var("GR_TEST_VAR");
        }
        assert_eq!(get("TEST_VAR"), None);
    }
}
