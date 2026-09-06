//! Domain matching: root + subdomain auto-cover; multi-root per site.
//! Admin input is NOT DNS-validated (product requirement).

use gr_abi::{host_matches_any_root, host_matches_root};

pub fn normalize_hostname(h: &str) -> String {
    h.trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Resolve which site_id owns this host, if any.
pub fn resolve_site_for_host(
    host: &str,
    roots: &[(String /*root*/, String /*site_id*/, bool /*enabled*/)],
) -> Option<String> {
    let host = normalize_hostname(host);
    let mut best: Option<(usize, String)> = None;
    for (root, site_id, enabled) in roots {
        if !*enabled {
            continue;
        }
        if host_matches_root(&host, root) {
            let specificity = root.len();
            if best.as_ref().map(|(s, _)| specificity > *s).unwrap_or(true) {
                best = Some((specificity, site_id.clone()));
            }
        }
    }
    best.map(|(_, s)| s)
}

pub fn host_allowed_for_site(host: &str, site_roots: &[String]) -> bool {
    host_matches_any_root(&normalize_hostname(host), site_roots)
}

/// FE / API gate: not a configured business host → treat as forbidden (403 path).
pub fn is_business_host(
    host: &str,
    roots: &[(String, String, bool)],
) -> bool {
    resolve_site_for_host(host, roots).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_root_and_subdomain() {
        let roots = vec![
            ("example.com".into(), "s1".into(), true),
            ("example.cn".into(), "s1".into(), true),
            ("other.com".into(), "s2".into(), true),
        ];
        assert_eq!(
            resolve_site_for_host("www.example.com", &roots).as_deref(),
            Some("s1")
        );
        assert_eq!(
            resolve_site_for_host("a.example.cn", &roots).as_deref(),
            Some("s1")
        );
        assert_eq!(
            resolve_site_for_host("other.com", &roots).as_deref(),
            Some("s2")
        );
        assert!(resolve_site_for_host("not-registered.test", &roots).is_none());
    }

    #[test]
    fn collect_disabled_root_does_not_resolve() {
        let roots = vec![
            ("fx-collect.gr.local".into(), "fx-collect".into(), false),
            ("shop.gr.local".into(), "shop".into(), true),
        ];
        assert!(
            resolve_site_for_host("fx-collect.gr.local", &roots).is_none(),
            "collect_enabled=false must not resolve a site"
        );
        assert_eq!(
            resolve_site_for_host("shop.gr.local", &roots).as_deref(),
            Some("shop")
        );
        assert!(!is_business_host("fx-collect.gr.local", &roots));
    }
}
