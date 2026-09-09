//! Network enrichment for gateway denorm (ASN / country / datacenter).
//!
//! Source order:
//! 1. CDN headers (caller)  
//! 2. MaxMind / DB-IP MMDB via `GR_GEOIP_ASN_MMDB` + `GR_GEOIP_COUNTRY_MMDB`
//!    or combined `GR_GEOIP_MMDB` (country-only fallback)  
//! 3. Built-in heuristics (private/loopback/coarse cloud)

use maxminddb::{geoip2, Reader};
use once_cell::sync::OnceCell;
use serde_json::{json, Map, Value};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct NetTags {
    pub asn: Option<String>,
    pub country: Option<String>,
    pub org: Option<String>,
    pub datacenter: Option<bool>,
    pub network_class: Option<String>,
    pub source: &'static str,
}

struct MmdbHolders {
    asn: Option<Reader<Vec<u8>>>,
    country: Option<Reader<Vec<u8>>>,
}

static MMDB: OnceCell<Mutex<MmdbHolders>> = OnceCell::new();

fn mmdb_holders() -> &'static Mutex<MmdbHolders> {
    MMDB.get_or_init(|| Mutex::new(load_mmdb_from_env()))
}

fn load_mmdb_from_env() -> MmdbHolders {
    // P0 运行时数据 (greenpng 1.0.8+ data_tree 随包分发): 解析顺序 —
    //   1. env GR_GEOIP_ASN_MMDB / GR_GEOIP_COUNTRY_MMDB (显式覆盖)
    //   2. GR_DATA_DIR/geo/dbip-*.mmdb (安装器 .env → systemd EnvironmentFile;
    //      开发仓 = 仓根 data/geo)
    //   3. cwd data/geo + 可执行文件相对 ../data/geo (无 env 姿态兜底)
    // 旧实现只有编译期 CARGO_MANIFEST_DIR 回退 — 安装机 (/opt/greenpng) 上
    // 该路径不存在, geoip 恒空 (178 实测 country/asn null 根因)。
    let data_geo_dir = || -> Option<PathBuf> {
        for base in data_dir_candidates() {
            let p = base.join("geo");
            if p.join("dbip-asn-lite.mmdb").is_file() || p.join("dbip-country-lite.mmdb").is_file()
            {
                return Some(p);
            }
        }
        None
    };
    let geo = data_geo_dir();
    let asn_path = gr_abi::env::get("GEOIP_ASN_MMDB")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            geo.as_ref().map(|p| p.join("dbip-asn-lite.mmdb").display().to_string())
        });
    let country_path = gr_abi::env::get("GEOIP_COUNTRY_MMDB")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            // Legacy alias: GEOIP_MMDB accepted (GR_/GR_/GR_ precedence inside get).
            geo.as_ref().map(|p| p.join("dbip-country-lite.mmdb").display().to_string())
        });
    let asn = asn_path.and_then(|p| match Reader::open_readfile(&p) {
        Ok(r) => {
            eprintln!("[net_enrich] ASN MMDB loaded path={p}");
            Some(r)
        }
        Err(e) => {
            eprintln!("[net_enrich] ASN MMDB open fail path={p} err={e}");
            None
        }
    });
    let country = country_path.and_then(|p| match Reader::open_readfile(&p) {
        Ok(r) => {
            eprintln!("[net_enrich] Country MMDB loaded path={p}");
            Some(r)
        }
        Err(e) => {
            eprintln!("[net_enrich] Country MMDB open fail path={p} err={e}");
            None
        }
    });
    if asn.is_none() && country.is_none() {
        eprintln!("[net_enrich] no geoip mmdb found (env GR_GEOIP_*_MMDB / data_dir geo/ both empty) — ASN/country 富化退内置启发式");
    }
    MmdbHolders { asn, country }
}

/// Candidate `data/` roots for runtime product data (geoip mmdb etc.).
/// Order: GR_DATA_DIR → cwd `data` → exe-dir relative `../data` (安装布局
/// bin/ + data/) → 编译期 workspace data/ (开发机)。
fn data_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = gr_abi::env::get("DATA_DIR").filter(|s| !s.trim().is_empty()) {
        out.push(PathBuf::from(d));
    }
    out.push(PathBuf::from("data"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            out.push(bin_dir.join("../data"));
            out.push(bin_dir.join("data"));
        }
    }
    out.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data"));
    out
}

/// Enrich fields when ASN/country still empty.
pub fn enrich_fields_if_empty(fields: &mut Map<String, Value>, client_ip: Option<&str>) {
    let has_asn = fields
        .get("server_asn")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let has_cc = fields
        .get("server_country")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let tags = classify_ip(client_ip);
    if !has_asn {
        if let Some(ref a) = tags.asn {
            fields.insert("server_asn".into(), json!(a));
            fields.insert("server_asn_source".into(), json!(tags.source));
        }
    }
    if !has_cc {
        if let Some(ref c) = tags.country {
            fields.insert("server_country".into(), json!(c));
            fields.insert("server_country_source".into(), json!(tags.source));
        }
    }
    if let Some(ref org) = tags.org {
        if !fields.contains_key("server_asn_org") {
            fields.insert("server_asn_org".into(), json!(org));
        }
    }
    if let Some(dc) = tags.datacenter {
        if !fields.contains_key("server_datacenter") {
            fields.insert("server_datacenter".into(), json!(dc));
        }
        fields.insert("server_datacenter_source".into(), json!(tags.source));
    }
    if let Some(ref cls) = tags.network_class {
        if !fields.contains_key("server_network_class") {
            fields.insert("server_network_class".into(), json!(cls));
        }
    }
}

pub fn classify_ip(ip_s: Option<&str>) -> NetTags {
    let Some(raw) = ip_s.map(str::trim).filter(|s| !s.is_empty()) else {
        return NetTags {
            network_class: Some("unknown".into()),
            source: "none",
            ..Default::default()
        };
    };
    let Ok(ip) = raw.parse::<IpAddr>() else {
        return NetTags {
            network_class: Some("invalid".into()),
            source: "parse_fail",
            ..Default::default()
        };
    };
    if ip.is_loopback() {
        return NetTags {
            network_class: Some("loopback".into()),
            datacenter: Some(false),
            source: "builtin_class",
            ..Default::default()
        };
    }
    if match ip {
        IpAddr::V4(v) => v.is_private() || v.is_link_local(),
        IpAddr::V6(v) => {
            let o = v.octets();
            (o[0] & 0xfe) == 0xfc || v.is_unicast_link_local()
        }
    } {
        return NetTags {
            network_class: Some("private".into()),
            datacenter: Some(false),
            source: "builtin_class",
            ..Default::default()
        };
    }

    // MMDB lookup (DB-IP / MaxMind GeoLite)
    if let Some(tags) = lookup_mmdb(ip) {
        return tags;
    }

    // Coarse cloud heuristics (challenge-only quality)
    if let IpAddr::V4(v4) = ip {
        let o = v4.octets();
        let (a, b) = (o[0], o[1]);
        let aws = matches!((a, b), (3, _) | (13, _) | (18, _) | (34, _) | (35, _) | (52, _) | (54, _));
        let azure = matches!((a, b), (20, _) | (40, _));
        let gcp = matches!((a, b), (34, 64..=128) | (35, 184..=191));
        if aws || azure || gcp {
            let label = if aws {
                "cloud_aws_heuristic"
            } else if gcp {
                "cloud_gcp_heuristic"
            } else {
                "cloud_azure_heuristic"
            };
            return NetTags {
                asn: Some(format!("HEUR:{label}")),
                datacenter: Some(true),
                network_class: Some("datacenter_heuristic".into()),
                source: "builtin_cloud_heuristic",
                ..Default::default()
            };
        }
    }
    NetTags {
        network_class: Some("public".into()),
        datacenter: Some(false),
        source: "builtin_class",
        ..Default::default()
    }
}

fn lookup_mmdb(ip: IpAddr) -> Option<NetTags> {
    let guard = mmdb_holders().lock().ok()?;
    let mut asn: Option<String> = None;
    let mut org: Option<String> = None;
    let mut country: Option<String> = None;
    let mut source = "mmdb";

    if let Some(ref reader) = guard.asn {
        // MaxMind ASN model
        if let Ok(rec) = reader.lookup::<geoip2::Asn>(ip) {
            if let Some(n) = rec.autonomous_system_number {
                asn = Some(format!("AS{n}"));
            }
            if let Some(o) = rec.autonomous_system_organization {
                org = Some(o.to_string());
            }
            source = "mmdb_asn";
        } else {
            // DB-IP free ASN often uses custom map — try generic JSON
            if let Ok(Some(v)) = reader.lookup::<Option<Value>>(ip) {
                if let Some(obj) = v.as_object() {
                    if let Some(n) = obj
                        .get("autonomous_system_number")
                        .or_else(|| obj.get("asn"))
                        .and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
                    {
                        asn = Some(format!("AS{n}"));
                    }
                    if let Some(o) = obj
                        .get("autonomous_system_organization")
                        .or_else(|| obj.get("as_name"))
                        .or_else(|| obj.get("organization"))
                        .and_then(|x| x.as_str())
                    {
                        org = Some(o.to_string());
                    }
                    source = "mmdb_asn_dbip";
                }
            }
        }
    }

    if let Some(ref reader) = guard.country {
        if let Ok(rec) = reader.lookup::<geoip2::Country>(ip) {
            if let Some(c) = rec
                .country
                .as_ref()
                .and_then(|c| c.iso_code)
                .or_else(|| rec.registered_country.as_ref().and_then(|c| c.iso_code))
            {
                country = Some(c.to_string());
                if source == "mmdb" {
                    source = "mmdb_country";
                } else if source.starts_with("mmdb_asn") {
                    source = "mmdb_asn_country";
                }
            }
        } else if let Ok(Some(v)) = reader.lookup::<Option<Value>>(ip) {
            // DB-IP country lite: { "country": { "iso_code": "US" } } or flat
            let cc = v
                .pointer("/country/iso_code")
                .or_else(|| v.get("country_code"))
                .or_else(|| v.get("country"))
                .and_then(|x| x.as_str());
            if let Some(c) = cc {
                if c.len() == 2 {
                    country = Some(c.to_uppercase());
                    source = if asn.is_some() {
                        "mmdb_asn_country"
                    } else {
                        "mmdb_country_dbip"
                    };
                }
            }
        }
    }

    if asn.is_none() && country.is_none() {
        return None;
    }
    let dc = org
        .as_ref()
        .map(|o| {
            let l = o.to_ascii_lowercase();
            l.contains("amazon")
                || l.contains("google")
                || l.contains("microsoft")
                || l.contains("digitalocean")
                || l.contains("ovh")
                || l.contains("hetzner")
                || l.contains("cloud")
                || l.contains("hosting")
                || l.contains("colo")
        })
        .unwrap_or(false);
    Some(NetTags {
        asn,
        country,
        org,
        datacenter: Some(dc),
        network_class: Some(if dc {
            "datacenter".into()
        } else {
            "public".into()
        }),
        source,
    })
}

/// JA4 ↔ User-Agent family mismatch (protocol vs claimed browser).
///
/// Uses JA4 structure `t{ver}{sni}{cipher_count}…_{cipher_hash}_{ext_hash}` plus optional
/// `protocol_engine` / ALPN hints when present in the JA4 string or companion fields.
pub fn ja4_ua_mismatch(ja4: Option<&str>, user_agent: Option<&str>) -> Option<&'static str> {
    ja4_ua_mismatch_ex(ja4, user_agent, None, None)
}

/// Extended: optional protocol_engine (from gateway) and alpn list.
pub fn ja4_ua_mismatch_ex(
    ja4: Option<&str>,
    user_agent: Option<&str>,
    protocol_engine: Option<&str>,
    alpn: Option<&str>,
) -> Option<&'static str> {
    let ua = user_agent?.to_ascii_lowercase();
    if ua.is_empty() {
        return None;
    }
    let ua_eng = ua_engine_family(&ua);
    let j = ja4.unwrap_or("").trim().to_ascii_lowercase();
    let pe = protocol_engine.unwrap_or("").to_ascii_lowercase();
    let al = alpn.unwrap_or("").to_ascii_lowercase();

    // Explicit protocol_engine vs UA (strongest when gateway tags engine)
    if !pe.is_empty() && pe != "unknown" && pe != "unknown_tls13" {
        let pe_fam = if pe.contains("firefox") || pe.contains("gecko") {
            "firefox"
        } else if pe.contains("safari") || pe.contains("webkit") || pe.contains("apple") {
            "safari"
        } else if pe.contains("chrome") || pe.contains("blink") || pe.contains("edge") {
            "chrome"
        } else {
            "other"
        };
        if pe_fam != "other" && ua_eng != "other" && pe_fam != ua_eng {
            return Some("protocol_engine_ua_family_mismatch");
        }
    }

    if j.is_empty() {
        return None;
    }

    // ALPN token inside JA4 (last part of a_ segment often encodes alpn chars)
    let ja4_says_h3 = j.contains("h3") || al.contains("h3");
    let ja4_says_h2 = j.contains("h2") || al.contains("h2");
    let ja4_says_h1 = j.contains("h1") || al.contains("http/1.1") || al.contains("http/1.0");

    // Parse JA4 first segment: t{tls}{sni}{cipher_count}{ext_count}{alpn}_…
    // e.g. t13d1516h2 → tls1.3, SNI present (d), 15 ciphers, 16 extensions, h2
    let (cipher_n, ext_n) = parse_ja4_counts(&j);

    // Firefox UA claiming HTTP/3-only chrome stacks is rare; chrome UA with ancient TLS is weak
    if ua_eng == "firefox" {
        // Synthetic / spoofed stacks sometimes copy chrome JA4 wholesale
        if j.contains("chrome") || pe.contains("chrome") {
            return Some("firefox_ua_chrome_tls_hint");
        }
        // Modern Chrome-class stacks often advertise 13–16 ciphers with grease; Firefox mobile differs
        if let Some(cn) = cipher_n {
            if cn >= 15 && ja4_says_h2 && pe.contains("blink") {
                return Some("firefox_ua_chrome_cipher_density");
            }
        }
    }
    if ua_eng == "safari" {
        if j.contains("chrome") || pe.contains("chrome") || pe.contains("blink") {
            return Some("safari_ua_chromeish_tls");
        }
        // Safari rarely advertises grease-heavy chrome fingerprints
        if j.contains("grease") && ja4_says_h2 && !ja4_says_h3 {
            return Some("safari_ua_grease_h2_tls");
        }
        // Safari TLS stacks typically carry fewer cipher suites than chrome 15+
        if let Some(cn) = cipher_n {
            if cn >= 15 && (pe.contains("chrome") || pe.contains("blink")) {
                return Some("safari_ua_high_cipher_chrome_tls");
            }
        }
    }
    if ua_eng == "chrome" || ua_eng == "edge" {
        // IE/Trident leftovers never; gecko token in TLS path is strong
        if pe.contains("gecko") || pe.contains("firefox") {
            return Some("chrome_ua_gecko_tls");
        }
        // Chrome claiming HTTP/1-only ancient stack while UA is modern chrome is soft mismatch
        if ja4_says_h1 && !ja4_says_h2 && !ja4_says_h3 {
            if let Some(cn) = cipher_n {
                if cn <= 5 {
                    return Some("chrome_ua_ancient_tls_stack");
                }
            }
        }
    }
    // Generic: protocol_engine family already handled above; also JA4 a-segment vs ALPN claim
    if ja4_says_h3 && al.contains("http/1") && !al.contains("h3") {
        return Some("ja4_h3_alpn_http1_claim");
    }
    // Extension density extreme outliers (bots / custom stacks)
    if let (Some(cn), Some(en)) = (cipher_n, ext_n) {
        if cn == 0 || en == 0 {
            return Some("ja4_zero_cipher_or_ext");
        }
    }
    None
}

/// Parse cipher/extension counts from JA4 first token (best-effort).
fn parse_ja4_counts(ja4: &str) -> (Option<u32>, Option<u32>) {
    // first segment before '_'
    let head = ja4.split('_').next().unwrap_or(ja4);
    // strip leading t13d / t12i / q13d etc → remaining digits pairs + optional alpn
    let bytes = head.as_bytes();
    if bytes.len() < 6 {
        return (None, None);
    }
    // find first digit run after protocol letter(s)
    let mut i = 0usize;
    while i < bytes.len() && !bytes[i].is_ascii_digit() {
        i += 1;
    }
    // skip tls version digits (usually 2: "13" or "12")
    let ver_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i - ver_start < 2 {
        return (None, None);
    }
    // optional sni flag d/i
    if i < bytes.len() && (bytes[i] == b'd' || bytes[i] == b'i') {
        i += 1;
    }
    // next 2 digits = cipher count, next 2 = ext count
    if i + 4 > bytes.len() {
        return (None, None);
    }
    let c1 = std::str::from_utf8(&bytes[i..i + 2])
        .ok()
        .and_then(|s| s.parse().ok());
    let e1 = std::str::from_utf8(&bytes[i + 2..i + 4])
        .ok()
        .and_then(|s| s.parse().ok());
    (c1, e1)
}

fn ua_engine_family(ua: &str) -> &'static str {
    if ua.contains("firefox/") || ua.contains("fxios") {
        "firefox"
    } else if ua.contains("edg/") || ua.contains("edgios") {
        "edge"
    } else if (ua.contains("safari/") && !ua.contains("chrome/") && !ua.contains("chromium"))
        || (ua.contains("iphone") && ua.contains("safari") && !ua.contains("crios"))
    {
        "safari"
    } else if ua.contains("chrome/")
        || ua.contains("crios")
        || ua.contains("chromium")
        || ua.contains("opr/")
    {
        "chrome"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_private() {
        let t = classify_ip(Some("127.0.0.1"));
        assert_eq!(t.network_class.as_deref(), Some("loopback"));
        let t2 = classify_ip(Some("10.0.0.5"));
        assert_eq!(t2.network_class.as_deref(), Some("private"));
    }

    #[test]
    fn enrich_writes_once() {
        let mut m = Map::new();
        enrich_fields_if_empty(&mut m, Some("127.0.0.1"));
        assert!(m.get("server_network_class").is_some());
        m.insert("server_asn".into(), json!("AS123"));
        enrich_fields_if_empty(&mut m, Some("8.8.8.8"));
        assert_eq!(m.get("server_asn").and_then(|v| v.as_str()), Some("AS123"));
    }

    #[test]
    fn mmdb_lookup_public_if_present() {
        // Google DNS — should resolve if mmdb downloaded
        let t = classify_ip(Some("8.8.8.8"));
        // Either mmdb country/asn or public fallback
        assert!(t.network_class.is_some());
        if t.source.starts_with("mmdb") {
            assert!(t.country.is_some() || t.asn.is_some(), "{t:?}");
        }
    }

    /// P0 (greenpng 1.0.8+ data_tree): 随包 dbip mmdb 经 data_dir 候选链
    /// (无 env 显式覆盖) 必须可真实打开 — 候选含编译期仓路径, dev 树与
    /// 公开扁平树 (crates/gr-probe-core → ../../../data/geo) 均在位。
    /// 装载链硬门: 178 1.0.7 曾因仅编译期单路径且安装机不存在而恒空
    /// (country/asn null 根因), 此测试防止数据树再被裁掉。
    #[test]
    fn shipped_mmdb_loads_via_data_dir_candidates() {
        let geo = data_dir_candidates()
            .into_iter()
            .map(|b| b.join("geo"))
            .find(|p| p.join("dbip-asn-lite.mmdb").is_file());
        let Some(geo) = geo else {
            panic!(
                "data/geo/dbip-asn-lite.mmdb not found via candidates {:?}",
                data_dir_candidates()
            );
        };
        let asn = Reader::open_readfile(geo.join("dbip-asn-lite.mmdb"));
        let country = Reader::open_readfile(geo.join("dbip-country-lite.mmdb"));
        assert!(asn.is_ok(), "asn mmdb must open ({:?}): {:?}", geo, asn.err());
        assert!(
            country.is_ok(),
            "country mmdb must open ({:?}): {:?}",
            geo,
            country.err()
        );
        // 真实查一次公网 IP — 8.8.8.8 应命中 (dbip lite 覆盖)
        let tags = lookup_mmdb("8.8.8.8".parse().unwrap());
        let t = tags.expect("mmdb lookup for 8.8.8.8");
        assert!(
            t.country.is_some() || t.asn.is_some(),
            "dbip lite must enrich 8.8.8.8"
        );
    }

    #[test]
    fn ja4_ua_protocol_engine_mismatch() {
        let h = ja4_ua_mismatch_ex(
            Some("t13d1516h2_8daaf6152771_b0da82dd1658"),
            Some("Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0"),
            Some("chrome_blink"),
            Some("h2"),
        );
        assert_eq!(h, Some("protocol_engine_ua_family_mismatch"));
    }

    #[test]
    fn ja4_parse_counts() {
        let (c, e) = parse_ja4_counts("t13d1516h2_8daaf6152771_b0da82dd1658");
        assert_eq!(c, Some(15));
        assert_eq!(e, Some(16));
    }

    #[test]
    fn ja4_zero_cipher_flag() {
        let h = ja4_ua_mismatch_ex(
            Some("t13d0000h2_deadbeef_cafebabe"),
            Some("Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36"),
            None,
            Some("h2"),
        );
        assert_eq!(h, Some("ja4_zero_cipher_or_ext"));
    }
}
