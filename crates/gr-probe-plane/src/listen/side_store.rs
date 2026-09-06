//! In-process side-channel sample store with **time-bounded, stronger-than-IP join**.
//!
//! iss/45 B-P0 + iss/46 + iss/50 G1:
//! - Prefer **IP + source port** (and optional dest port / conn id) within TTL.
//! - IP-only matches are labeled **weak** (`ip_ttl_window_weak`).
//! - Weak joins **never** allow commercial-identity hard-merge; they may still
//!   enrich diagnostic/conf fields with an explicit policy flag.
//! - Strong joins enrich protocol fields for brain/xsrc without promoting to
//!   commercial device body material.

use gr_probe_net::{QuicInitialInfo, TcpSynInfo, WebrtcSideInfo};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX: usize = 128;
/// Join freshness window (ms). Older side samples are ignored for enrich.
const SIDE_TTL_MS: i64 = 30_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Request-side association scope for side-channel join.
#[derive(Debug, Clone, Default)]
pub struct JoinScope {
    pub client_ip: Option<String>,
    /// Client/source port of the HTTP(S)/H3 request (from peer or PROXY).
    pub src_port: Option<u16>,
    /// Local/destination port of the request (service bind port) when known.
    pub dst_port: Option<u16>,
    /// Optional connection/session token (e.g. QUIC DCID prefix, session id).
    pub conn_token: Option<String>,
}

impl JoinScope {
    pub fn from_ip(ip: Option<&str>) -> Self {
        Self {
            client_ip: ip.filter(|s| !s.is_empty()).map(|s| s.to_string()),
            ..Default::default()
        }
    }
}

/// How strongly the side sample is tied to this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JoinStrength {
    None = 0,
    /// Source IP + TTL only — NAT multi-visitor risk.
    WeakIpTtl = 1,
    /// IP + source port within TTL.
    StrongIpPort = 2,
    /// IP + source port + dest port or conn token within TTL.
    StrongIpConn = 3,
}

impl JoinStrength {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WeakIpTtl => "ip_ttl_window_weak",
            Self::StrongIpPort => "ip_port_ttl_strong",
            Self::StrongIpConn => "ip_conn_ttl_strong",
        }
    }

    pub fn is_strong(self) -> bool {
        matches!(self, Self::StrongIpPort | Self::StrongIpConn)
    }

    /// Side-channel protocol samples must **never** hard-merge commercial device ids.
    /// Weak joins are additionally barred from soft hard-link promotion.
    pub fn commercial_merge_allowed(self) -> bool {
        false
    }

    pub fn soft_hard_link_allowed(self) -> bool {
        self.is_strong()
    }
}

/// Score a stored sample against the request scope. Higher is better; 0 = no match.
fn score_ip_port(
    sample_ip: Option<&str>,
    sample_sport: Option<u16>,
    sample_dport: Option<u16>,
    sample_conn: Option<&str>,
    scope: &JoinScope,
) -> (u32, JoinStrength) {
    let Some(ref want_ip) = scope.client_ip else {
        return (0, JoinStrength::None);
    };
    if sample_ip != Some(want_ip.as_str()) {
        return (0, JoinStrength::None);
    }
    let mut score = 10u32; // IP match base
    let mut strength = JoinStrength::WeakIpTtl;

    let port_match = match (scope.src_port, sample_sport) {
        (Some(a), Some(b)) if a > 0 && b > 0 && a == b => true,
        _ => false,
    };
    if port_match {
        score += 50;
        strength = JoinStrength::StrongIpPort;
    } else if scope.src_port.is_some() && sample_sport.is_some() {
        // Explicit port mismatch under same IP: do not join this sample.
        return (0, JoinStrength::None);
    }

    let dport_match = match (scope.dst_port, sample_dport) {
        (Some(a), Some(b)) if a > 0 && b > 0 && a == b => true,
        _ => false,
    };
    if dport_match && strength.is_strong() {
        score += 20;
        strength = JoinStrength::StrongIpConn;
    }

    if let (Some(want), Some(got)) = (scope.conn_token.as_deref(), sample_conn) {
        if !want.is_empty() && want == got {
            score += 30;
            if strength.is_strong() {
                strength = JoinStrength::StrongIpConn;
            } else {
                // conn token alone under IP is stronger than bare IP
                strength = JoinStrength::StrongIpPort;
                score += 40;
            }
        }
    }

    (score, strength)
}

/// JA4T-style TCP fingerprint: `{window}_{options}_{mss}_{wscale}` (FoxIO).
fn format_ja4t(s: &TcpSynInfo) -> String {
    let win = s
        .tcp_window
        .map(|w| w.to_string())
        .unwrap_or_else(|| "0".into());
    let opts = s
        .option_order_str
        .clone()
        .unwrap_or_else(|| {
            s.option_order
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("-")
        });
    let mss = s
        .tcp_mss
        .map(|m| m.to_string())
        .unwrap_or_else(|| "0".into());
    let ws = s
        .tcp_wscale
        .map(|w| w.to_string())
        .unwrap_or_else(|| "0".into());
    format!("{win}_{opts}_{mss}_{ws}")
}

/// HTTP/3 application-layer sample (full H3 server path).
#[derive(Debug, Clone, Default)]
pub struct H3AppInfo {
    pub present: bool,
    pub src_ip: String,
    pub src_port: u16,
    pub alpn: String,
    pub method: String,
    pub path: String,
    pub pseudo_order: String,
    pub settings_fp: String,
    pub rtt_ms: Option<f64>,
    pub frames_rx: Option<u64>,
    pub udp_rx: Option<u64>,
    pub note: String,
}

struct Timed<T> {
    seen_at_ms: i64,
    info: T,
}

struct Store {
    quic: VecDeque<Timed<QuicInitialInfo>>,
    syn: VecDeque<Timed<TcpSynInfo>>,
    webrtc: VecDeque<Timed<WebrtcSideInfo>>,
    h3_app: VecDeque<Timed<H3AppInfo>>,
}

fn store() -> &'static Mutex<Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(Store {
            quic: VecDeque::new(),
            syn: VecDeque::new(),
            webrtc: VecDeque::new(),
            h3_app: VecDeque::new(),
        })
    })
}

pub fn push_h3_app(info: H3AppInfo) {
    if !info.present {
        return;
    }
    if let Ok(mut g) = store().lock() {
        g.h3_app.push_back(Timed {
            seen_at_ms: now_ms(),
            info,
        });
        while g.h3_app.len() > MAX {
            g.h3_app.pop_front();
        }
    }
}

pub fn latest_h3_app_for_ip(ip: &str) -> Option<H3AppInfo> {
    pick_h3(&JoinScope::from_ip(Some(ip))).map(|(h, _)| h)
}

fn pick_h3(scope: &JoinScope) -> Option<(H3AppInfo, JoinStrength)> {
    let g = store().lock().ok()?;
    let cutoff = now_ms() - SIDE_TTL_MS;
    let mut best: Option<(u32, JoinStrength, &Timed<H3AppInfo>)> = None;
    for h in g.h3_app.iter().rev() {
        if h.seen_at_ms < cutoff {
            continue;
        }
        let sport = if h.info.src_port > 0 {
            Some(h.info.src_port)
        } else {
            None
        };
        let (sc, st) = score_ip_port(
            Some(h.info.src_ip.as_str()),
            sport,
            None,
            None,
            scope,
        );
        if sc == 0 {
            continue;
        }
        match best {
            None => best = Some((sc, st, h)),
            Some((bsc, _, _)) if sc > bsc => best = Some((sc, st, h)),
            Some((bsc, bst, _)) if sc == bsc && st > bst => best = Some((sc, st, h)),
            _ => {}
        }
    }
    best.map(|(_, st, t)| (t.info.clone(), st))
}

/// Inject H3 app headers from a **known** sample (this connection). Prefer over lookup.
pub fn inject_h3_app_headers_from_info(
    headers: &mut std::collections::HashMap<String, String>,
    info: &H3AppInfo,
) {
    if !info.present {
        return;
    }
    headers.insert("x-gr-h3-app".into(), "1".into());
    headers.insert("x-gr-h3-alpn".into(), info.alpn.clone());
    headers.insert("x-gr-h3-pseudo-order".into(), info.pseudo_order.clone());
    headers.insert("x-gr-h3-settings-fp".into(), info.settings_fp.clone());
    // This connection's own sample is always conn-strong (no NAT cross-join).
    headers.insert(
        "x-gr-side-join-strength".into(),
        JoinStrength::StrongIpConn.as_str().into(),
    );
    headers.insert("x-gr-h3-join-strength".into(), JoinStrength::StrongIpConn.as_str().into());
    if info.src_port > 0 {
        headers.insert("x-gr-h3-src-port".into(), info.src_port.to_string());
        headers.insert("x-gr-peer-port".into(), info.src_port.to_string());
    }
    if !info.src_ip.is_empty() {
        headers.insert("x-gr-h3-src-ip".into(), info.src_ip.clone());
    }
    if let Some(rtt) = info.rtt_ms {
        headers.insert("x-gr-h3-rtt-ms".into(), format!("{rtt:.2}"));
    }
    if !info.note.is_empty() {
        headers.insert("x-gr-h3-note".into(), info.note.clone());
    }
}

/// Lookup + inject using full join scope (IP+port when known). iss/50 G1.
pub fn inject_h3_app_headers_scoped(
    headers: &mut std::collections::HashMap<String, String>,
    scope: &JoinScope,
) {
    let Some((h, strength)) = pick_h3(scope) else {
        return;
    };
    headers.insert("x-gr-h3-app".into(), "1".into());
    headers.insert("x-gr-h3-alpn".into(), h.alpn);
    headers.insert("x-gr-h3-pseudo-order".into(), h.pseudo_order);
    headers.insert("x-gr-h3-settings-fp".into(), h.settings_fp);
    headers.insert("x-gr-side-join-strength".into(), strength.as_str().into());
    headers.insert("x-gr-h3-join-strength".into(), strength.as_str().into());
    if h.src_port > 0 {
        headers.insert("x-gr-h3-src-port".into(), h.src_port.to_string());
    }
    if let Some(rtt) = h.rtt_ms {
        headers.insert("x-gr-h3-rtt-ms".into(), format!("{rtt:.2}"));
    }
}

/// Backward-compat IP-only inject — **weak**. Prefer scoped / from_info.
#[deprecated(note = "use inject_h3_app_headers_scoped or inject_h3_app_headers_from_info")]
pub fn inject_h3_app_headers(headers: &mut std::collections::HashMap<String, String>, ip: &str) {
    inject_h3_app_headers_scoped(headers, &JoinScope::from_ip(Some(ip)));
}

pub fn push_quic(info: QuicInitialInfo) {
    if let Ok(mut g) = store().lock() {
        // Prefer samples that already have JA4 / richer crypto when same src IP
        if let Some(ip) = info.src_ip.clone() {
            if info.tls_ja4.is_some() {
                g.quic.retain(|q| {
                    !(q.info.src_ip.as_deref() == Some(ip.as_str()) && q.info.tls_ja4.is_none())
                });
            }
        }
        g.quic.push_back(Timed {
            seen_at_ms: now_ms(),
            info,
        });
        while g.quic.len() > MAX {
            if let Some(pos) = g.quic.iter().position(|q| q.info.tls_ja4.is_none()) {
                g.quic.remove(pos);
            } else {
                g.quic.pop_front();
            }
        }
    }
}

pub fn push_syn(info: TcpSynInfo) {
    if !info.parse_ok {
        return;
    }
    if let Ok(mut g) = store().lock() {
        g.syn.push_back(Timed {
            seen_at_ms: now_ms(),
            info,
        });
        while g.syn.len() > MAX {
            g.syn.pop_front();
        }
    }
}

pub fn push_webrtc(info: WebrtcSideInfo) {
    if !info.present {
        return;
    }
    if let Ok(mut g) = store().lock() {
        g.webrtc.push_back(Timed {
            seen_at_ms: now_ms(),
            info,
        });
        while g.webrtc.len() > MAX {
            g.webrtc.pop_front();
        }
    }
}

pub fn latest_quic_for_ip(ip: &str) -> Option<QuicInitialInfo> {
    pick_quic(&JoinScope::from_ip(Some(ip))).map(|(q, _)| q)
}

fn pick_quic(scope: &JoinScope) -> Option<(QuicInitialInfo, JoinStrength)> {
    let g = store().lock().ok()?;
    let cutoff = now_ms() - SIDE_TTL_MS;
    let mut best: Option<(u32, JoinStrength, &Timed<QuicInitialInfo>)> = None;
    for q in g.quic.iter().rev() {
        if q.seen_at_ms < cutoff {
            continue;
        }
        let conn = q.info.dcid_hex.as_deref();
        let (mut sc, st) = score_ip_port(
            q.info.src_ip.as_deref(),
            q.info.src_port,
            None,
            conn,
            scope,
        );
        if sc == 0 {
            continue;
        }
        // Prefer richer crypto samples at equal join strength.
        if q.info.tls_ja4.is_some() {
            sc += 5;
        }
        sc += (q.info.crypto_len.unwrap_or(0).min(1000) / 100) as u32;
        match best {
            None => best = Some((sc, st, q)),
            Some((bsc, bst, _)) if sc > bsc || (sc == bsc && st > bst) => {
                best = Some((sc, st, q));
            }
            _ => {}
        }
    }
    best.map(|(_, st, t)| (t.info.clone(), st))
}

pub fn latest_syn_for_ip(ip: &str) -> Option<TcpSynInfo> {
    pick_syn(&JoinScope::from_ip(Some(ip))).map(|(s, _)| s)
}

fn pick_syn(scope: &JoinScope) -> Option<(TcpSynInfo, JoinStrength)> {
    let g = store().lock().ok()?;
    let cutoff = now_ms() - SIDE_TTL_MS;
    let mut best: Option<(u32, JoinStrength, &Timed<TcpSynInfo>)> = None;
    for s in g.syn.iter().rev() {
        if s.seen_at_ms < cutoff {
            continue;
        }
        let (sc, st) = score_ip_port(
            s.info.src_ip.as_deref(),
            s.info.src_port,
            s.info.dst_port,
            None,
            scope,
        );
        if sc == 0 {
            continue;
        }
        match best {
            None => best = Some((sc, st, s)),
            Some((bsc, bst, _)) if sc > bsc || (sc == bsc && st > bst) => {
                best = Some((sc, st, s));
            }
            _ => {}
        }
    }
    best.map(|(_, st, t)| (t.info.clone(), st))
}

pub fn latest_webrtc_for_ip(ip: &str) -> Option<WebrtcSideInfo> {
    pick_webrtc(&JoinScope::from_ip(Some(ip))).map(|(w, _)| w)
}

fn pick_webrtc(scope: &JoinScope) -> Option<(WebrtcSideInfo, JoinStrength)> {
    let g = store().lock().ok()?;
    let cutoff = now_ms() - SIDE_TTL_MS;
    let mut best: Option<(u32, JoinStrength, &Timed<WebrtcSideInfo>)> = None;
    for w in g.webrtc.iter().rev() {
        if w.seen_at_ms < cutoff {
            continue;
        }
        let (sc, st) = score_ip_port(
            w.info.src_ip.as_deref(),
            w.info.src_port,
            w.info.dst_port,
            None,
            scope,
        );
        if sc == 0 {
            continue;
        }
        match best {
            None => best = Some((sc, st, w)),
            Some((bsc, bst, _)) if sc > bsc || (sc == bsc && st > bst) => {
                best = Some((sc, st, w));
            }
            _ => {}
        }
    }
    best.map(|(_, st, t)| (t.info.clone(), st))
}

/// Backward-compatible IP-only enrich (always weak unless samples carry no port).
pub fn enrich_protocol_fields(fields: &mut Map<String, Value>, client_ip: Option<&str>) {
    enrich_protocol_fields_scoped(fields, &JoinScope::from_ip(client_ip));
}

/// Merge side-channel observations using a full join scope (iss/50 G1).
///
/// Exports full QUIC TP / key_share / SYN p0f+JA4T with TTL + strength-aware join.
pub fn enrich_protocol_fields_scoped(fields: &mut Map<String, Value>, scope: &JoinScope) {
    let Some(ip) = scope.client_ip.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    let _ = ip;
    fields.insert("side_join_ttl_ms".into(), json!(SIDE_TTL_MS));
    fields.insert("side_join_algo".into(), json!("side_join_v2_ip_port_ttl"));
    // iss/46 M0(d): association metadata on side observations
    fields.insert("side_source".into(), json!("side_channel_listen"));
    fields.insert(
        "side_association_level".into(),
        json!("network_protocol_enrichment"),
    );
    fields.insert("side_seen_at_ms".into(), json!(now_ms()));
    fields.insert(
        "side_expiry_ms".into(),
        json!(now_ms() + SIDE_TTL_MS),
    );
    fields.insert(
        "side_join_scope_src_port".into(),
        json!(scope.src_port),
    );
    fields.insert(
        "side_join_scope_dst_port".into(),
        json!(scope.dst_port),
    );

    let mut best_strength = JoinStrength::None;
    let mut any = false;

    if let Some((q, st)) = pick_quic(scope) {
        any = true;
        if st > best_strength {
            best_strength = st;
        }
        fields.insert("quic_side_join_strength".into(), json!(st.as_str()));
        fields.insert("quic_listen_present".into(), json!(true));
        if let Some(ref j) = q.tls_ja4 {
            // Prefer TCP JA4 if already set; still record H3 path
            fields.insert("quic_tls_ja4".into(), json!(j));
            if !fields.contains_key("ja4") {
                fields.insert("ja4".into(), json!(j));
                fields.insert("tls_ja4".into(), json!(j));
                fields.insert("protocol_fp_source".into(), json!("gr_quic_listen"));
                fields.insert("tls_fingerprint_available".into(), json!(true));
            }
        }
        if let Some(ref j3) = q.tls_ja3_hash {
            fields.insert("quic_tls_ja3_hash".into(), json!(j3));
        }
        if let Some(ref fp) = q.quic_fp {
            fields.insert("quic_fp".into(), json!(fp));
            fields.insert("quic_initial_fp".into(), json!(fp));
        }
        if let Some(ref h) = q.quic_fp_hash {
            fields.insert("quic_fp_hash".into(), json!(h));
        }
        if let Some(ref sni) = q.sni {
            fields.insert("quic_sni".into(), json!(sni));
        }
        if !q.alpn.is_empty() {
            fields.insert("quic_alpn".into(), json!(q.alpn.join(",")));
        }
        fields.insert("quic_aead_ok".into(), json!(q.aead_ok));
        fields.insert("quic_retry".into(), json!(q.is_retry));
        fields.insert(
            "quic_version_label".into(),
            json!(q.version_label.clone().unwrap_or_default()),
        );
        if let Some(v) = q.version {
            fields.insert("quic_version".into(), json!(format!("{v:#x}")));
        }
        if let Some(dl) = q.dcid_len {
            fields.insert("quic_dcid_len".into(), json!(dl));
        }
        if let Some(sl) = q.scid_len {
            fields.insert("quic_scid_len".into(), json!(sl));
        }
        // Full key_share + transport params export (iss/45 B1)
        if !q.key_share_groups.is_empty() {
            fields.insert("quic_key_share_groups".into(), json!(q.key_share_groups.clone()));
            fields.insert(
                "quic_key_share_groups_hex".into(),
                json!(q.key_share_groups_hex.clone()),
            );
        }
        if !q.psk_modes_names.is_empty() {
            fields.insert("quic_psk_modes".into(), json!(q.psk_modes_names.join(",")));
        }
        fields.insert("quic_has_ech".into(), json!(q.has_ech));
        if let Some(ref sum) = q.transport_params_summary {
            fields.insert("quic_tp_summary".into(), json!(sum));
            fields.insert("quic_tp_hash".into(), json!({
                // short stable id from summary
                "v": sum.chars().take(64).collect::<String>()
            }));
        }
        if !q.transport_params_map.is_empty() {
            // Flatten common TP names into product fields when present
            for (k, v) in &q.transport_params_map {
                let key = format!("quic_tp_{}", k.replace('-', "_"));
                fields.insert(key, json!(v));
            }
            fields.insert(
                "quic_tp_map".into(),
                json!(q.transport_params_map.clone()),
            );
        }
        if let Some(tj) = fields.get("ja4").and_then(|v| v.as_str()).map(|s| s.to_string()) {
            if let Some(ref qj) = q.tls_ja4 {
                if !tj.is_empty() && !qj.is_empty() {
                    fields.insert("protocol_tcp_quic_agree".into(), json!(tj == *qj));
                }
            }
        }
        // Weak join: mark protocol fps as non-identity so soft paths cannot hard-merge.
        if !st.is_strong() {
            fields.insert("quic_side_join_weak".into(), json!(true));
        }
    }
    if let Some((s, st)) = pick_syn(scope) {
        any = true;
        if st > best_strength {
            best_strength = st;
        }
        fields.insert("tcp_syn_side_join_strength".into(), json!(st.as_str()));
        fields.insert("tcp_syn_present".into(), json!(true));
        if let Some(ttl) = s.ip_ttl {
            fields.insert("tcp_syn_ttl".into(), json!(ttl));
        }
        if let Some(w) = s.tcp_window {
            fields.insert("tcp_syn_window".into(), json!(w));
        }
        if let Some(ws) = s.tcp_window_scaled {
            fields.insert("tcp_syn_window_scaled".into(), json!(ws));
        }
        if let Some(mss) = s.tcp_mss {
            fields.insert("tcp_syn_mss".into(), json!(mss));
        }
        if let Some(ws) = s.tcp_wscale {
            fields.insert("tcp_syn_wscale".into(), json!(ws));
        }
        fields.insert("tcp_syn_sack_perm".into(), json!(s.tcp_sack_ok));
        fields.insert("tcp_syn_ts".into(), json!(s.tcp_timestamp));
        if let Some(ts) = s.tcp_ts_val {
            fields.insert("tcp_syn_ts_val".into(), json!(ts));
        }
        if let Some(tos) = s.ip_tos {
            fields.insert("tcp_syn_tos".into(), json!(tos));
        }
        if let Some(ecn) = s.ip_ecn {
            fields.insert("tcp_syn_ip_ecn".into(), json!(ecn));
        }
        if let Some(ipid) = s.ip_id {
            fields.insert("tcp_syn_ip_id".into(), json!(ipid));
        }
        if let Some(ref p0f) = s.p0f_sig {
            fields.insert("tcp_syn_p0f_sig".into(), json!(p0f));
        }
        if let Some(ref ph) = s.p0f_hash {
            fields.insert("tcp_syn_p0f_hash".into(), json!(ph));
        }
        if let Some(ref oo) = s.option_order_str {
            fields.insert("tcp_syn_option_order".into(), json!(oo));
        }
        // JA4T format from already-parsed materials (iss/45 B4)
        let ja4t = format_ja4t(&s);
        fields.insert("ja4t".into(), json!(ja4t));
        fields.insert("tcp_syn_ja4t".into(), json!(format_ja4t(&s)));
        // Compact option signature for brain/xsrc
        let sig = format!(
            "mss={}|ws={}|sack={}|ts={}|ttl={}",
            s.tcp_mss.map(|x| x.to_string()).unwrap_or_else(|| "-".into()),
            s.tcp_wscale.map(|x| x.to_string()).unwrap_or_else(|| "-".into()),
            s.tcp_sack_ok as u8,
            s.tcp_timestamp as u8,
            s.ip_ttl.map(|x| x.to_string()).unwrap_or_else(|| "-".into()),
        );
        fields.insert("tcp_syn_options_sig".into(), json!(sig));
        if let Some(ref fl) = s.tcp_flags_str {
            fields.insert("tcp_syn_flags".into(), json!(fl));
        }
        if !st.is_strong() {
            fields.insert("tcp_syn_side_join_weak".into(), json!(true));
        }
    }
    if let Some((w, st)) = pick_webrtc(scope) {
        any = true;
        if st > best_strength {
            best_strength = st;
        }
        fields.insert("webrtc_side_join_strength".into(), json!(st.as_str()));
        fields.insert("webrtc_side_present".into(), json!(true));
        fields.insert("webrtc_side_kind".into(), json!(w.kind.clone()));
        if w.is_stun {
            fields.insert("webrtc_stun".into(), json!(true));
        }
        if w.is_dtls_client_hello {
            fields.insert("webrtc_dtls_hello".into(), json!(true));
            if let Some(ref j) = w.dtls_ja4 {
                fields.insert("dtls_ja4".into(), json!(j));
            }
        }
        if !st.is_strong() {
            fields.insert("webrtc_side_join_weak".into(), json!(true));
        }
    }
    if let Some((h, st)) = pick_h3(scope) {
        any = true;
        if st > best_strength {
            best_strength = st;
        }
        fields.insert("h3_side_join_strength".into(), json!(st.as_str()));
        fields.insert("h3_app_present".into(), json!(true));
        fields.insert("h3_alpn".into(), json!(h.alpn));
        fields.insert("h3_pseudo_order".into(), json!(h.pseudo_order));
        fields.insert("h3_settings_fp".into(), json!(h.settings_fp));
        fields.insert("h3_method".into(), json!(h.method));
        fields.insert("h3_path".into(), json!(h.path));
        if h.src_port > 0 {
            fields.insert("h3_src_port".into(), json!(h.src_port));
        }
        if !h.note.is_empty() {
            fields.insert("h3_note".into(), json!(h.note));
        }
        fields.insert("protocol_fp_source".into(), json!("gr_h3_app"));
        if let Some(rtt) = h.rtt_ms {
            fields.insert("h3_rtt_ms".into(), json!(rtt));
        }
        if let Some(n) = h.frames_rx {
            fields.insert("h3_frames_rx".into(), json!(n));
        }
        if let Some(n) = h.udp_rx {
            fields.insert("h3_udp_rx".into(), json!(n));
        }
        if !st.is_strong() {
            fields.insert("h3_side_join_weak".into(), json!(true));
        }
    }

    if !any {
        best_strength = JoinStrength::None;
    } else if best_strength == JoinStrength::None {
        best_strength = JoinStrength::WeakIpTtl;
    }

    fields.insert(
        "side_join_strength".into(),
        json!(best_strength.as_str()),
    );
    fields.insert(
        "side_join_commercial_merge_allowed".into(),
        json!(best_strength.commercial_merge_allowed()),
    );
    fields.insert(
        "side_join_soft_hard_link_allowed".into(),
        json!(best_strength.soft_hard_link_allowed()),
    );
    fields.insert(
        "side_join_is_strong".into(),
        json!(best_strength.is_strong()),
    );
    // Explicit policy: side-channel never contributes commercial device body.
    fields.insert(
        "side_join_policy".into(),
        json!({
            "commercial_device_hard_merge": false,
            "weak_join_soft_hard_link": false,
            "strong_join_soft_hard_link": best_strength.is_strong(),
            "enrich_ok": any,
            "note": "NAT-safe: request port must match sample port when both known; IP-only is weak",
        }),
    );
    let ip_only = matches!(best_strength, JoinStrength::WeakIpTtl | JoinStrength::None);
    let truth = if !any || best_strength == JoinStrength::None {
        "absent"
    } else if ip_only {
        "inferred_weak"
    } else {
        "observed_connection"
    };
    fields.insert(
        "s0_observation".into(),
        json!({
            "truth_level": truth,
            "availability": if any { "observed" } else { "absent" },
            "scope": "connection",
            "join_strength": best_strength.as_str(),
            "commercial_eligible": false,
            "commercial_eligible_reason": if ip_only { "ip_only_join_weak" } else { "side_channel_protocol_not_commercial_mint" },
            "source_kind": "gateway",
        }),
    );
    fields.insert("s0_truth_level".into(), json!(truth));
    fields.insert("s0_commercial_eligible".into(), json!(false));
    // H1/H2: mark partial H2/H3 pseudo/priority as diagnostic-only after side enrich.
    gr_probe_core::annotate_protocol_export_honesty(fields);
}

/// Test/lab helper: wipe in-process side samples.
#[cfg(test)]
pub fn test_clear_store() {
    if let Ok(mut g) = store().lock() {
        g.quic.clear();
        g.syn.clear();
        g.webrtc.clear();
        g.h3_app.clear();
    }
}

pub fn recent_side_summary() -> Value {
    let g = store().lock().ok();
    match g {
        Some(g) => {
            let last_quic = g.quic.back().map(|t| {
                let q = &t.info;
                json!({
                    "seen_at_ms": t.seen_at_ms,
                    "src_ip": q.src_ip,
                    "aead_ok": q.aead_ok,
                    "tls_ja4": q.tls_ja4,
                    "quic_fp": q.quic_fp,
                    "version_label": q.version_label,
                    "crypto_len": q.crypto_len,
                    "key_share_groups": q.key_share_groups,
                    "tp_summary": q.transport_params_summary,
                    "notes": q.notes.iter().rev().take(4).cloned().collect::<Vec<_>>(),
                })
            });
            let last_syn = g.syn.back().map(|t| {
                let s = &t.info;
                json!({
                    "seen_at_ms": t.seen_at_ms,
                    "src_ip": s.src_ip,
                    "ttl": s.ip_ttl,
                    "window": s.tcp_window,
                    "mss": s.tcp_mss,
                    "options": s.option_order_str,
                    "p0f_sig": s.p0f_sig,
                    "ja4t": format_ja4t(s),
                })
            });
            let last_webrtc = g.webrtc.back().map(|t| {
                let w = &t.info;
                json!({
                    "seen_at_ms": t.seen_at_ms,
                    "src_ip": w.src_ip,
                    "kind": w.kind,
                    "is_stun": w.is_stun,
                    "is_dtls": w.is_dtls_client_hello,
                    "dtls_ja4": w.dtls_ja4,
                })
            });
            let last_h3 = g.h3_app.back().map(|t| {
                let h = &t.info;
                json!({
                    "seen_at_ms": t.seen_at_ms,
                    "src_ip": h.src_ip,
                    "src_port": h.src_port,
                    "alpn": h.alpn,
                    "path": h.path,
                    "method": h.method,
                    "pseudo_order": h.pseudo_order,
                    "settings_fp": h.settings_fp,
                    "rtt_ms": h.rtt_ms,
                    "note": h.note,
                })
            });
            json!({
                "quic_n": g.quic.len(),
                "syn_n": g.syn.len(),
                "webrtc_n": g.webrtc.len(),
                "h3_app_n": g.h3_app.len(),
                "side_ttl_ms": SIDE_TTL_MS,
                "last_quic": last_quic,
                "last_syn": last_syn,
                "last_webrtc": last_webrtc,
                "last_h3_app": last_h3,
            })
        }
        None => json!({}),
    }
}

/// Lab/CI inject: structured side-channel samples without CAP_NET_RAW / real UDP peers.
/// Gated by env `GR_SIDE_LAB=1` (checked by HTTP layer).
pub fn lab_inject(kind: &str, body: &Value) -> Result<Value, String> {
    let src_ip = body
        .get("src_ip")
        .and_then(|v| v.as_str())
        .unwrap_or("127.0.0.1")
        .to_string();
    match kind {
        "syn" | "tcp_syn" => {
            let info = TcpSynInfo {
                parse_ok: true,
                src_ip: Some(src_ip.clone()),
                src_port: body
                    .get("src_port")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u16)
                    .or(Some(40000)),
                dst_port: body
                    .get("dst_port")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u16)
                    .or(Some(28780)),
                ip_ttl: body
                    .get("ttl")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u8)
                    .or(Some(64)),
                tcp_window: body
                    .get("window")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u16)
                    .or(Some(65535)),
                tcp_mss: body
                    .get("mss")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u16)
                    .or(Some(1460)),
                tcp_wscale: body
                    .get("wscale")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u8)
                    .or(Some(7)),
                tcp_sack_ok: body
                    .get("sack_ok")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                tcp_timestamp: body
                    .get("timestamp")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                tcp_flags: Some(0x02),
                tcp_flags_str: Some("S".into()),
                option_order_str: Some("mss,sack,ts,ws".into()),
                ..Default::default()
            };
            push_syn(info);
            Ok(json!({"ok": true, "kind": "syn", "src_ip": src_ip}))
        }
        "quic" => {
            let info = QuicInitialInfo {
                parse_ok: true,
                src_ip: Some(src_ip.clone()),
                src_port: body
                    .get("src_port")
                    .and_then(|v| v.as_u64())
                    .map(|u| u as u16),
                aead_ok: body
                    .get("aead_ok")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                tls_ja4: body
                    .get("tls_ja4")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                quic_fp: body
                    .get("quic_fp")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| Some(format!("lab_quic_{src_ip}"))),
                quic_fp_hash: body
                    .get("quic_fp_hash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                dcid_hex: body
                    .get("dcid_hex")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                version_label: Some(
                    body.get("version_label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("QUIC v1")
                        .to_string(),
                ),
                ..Default::default()
            };
            push_quic(info);
            Ok(json!({"ok": true, "kind": "quic", "src_ip": src_ip}))
        }
        "webrtc" | "stun" | "dtls" => {
            let is_dtls = kind == "dtls"
                || body
                    .get("is_dtls")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            let sport = body
                .get("src_port")
                .and_then(|v| v.as_u64())
                .unwrap_or(40000) as u16;
            let info = if is_dtls {
                WebrtcSideInfo::from_dtls_hello_fp(
                    &src_ip,
                    sport,
                    body.get("dtls_ja4")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    None,
                )
            } else {
                let mut stun = gr_probe_net::StunInfo {
                    parse_ok: true,
                    is_binding_request: true,
                    message_type_name: Some("Binding Request".into()),
                    ..Default::default()
                };
                stun.has_username = body
                    .get("ice")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                WebrtcSideInfo::from_stun(stun, &src_ip, sport)
            };
            push_webrtc(info);
            Ok(json!({"ok": true, "kind": kind, "src_ip": src_ip}))
        }
        _ => Err(format!("unknown kind={kind}; use syn|quic|stun|dtls|webrtc")),
    }
}

pub fn side_lab_enabled() -> bool {
    matches!(
        gr_abi::env::get("SIDE_LAB").as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Global side store is process-wide; serialize tests that mutate it.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push_syn_ports(ip: &str, sport: u16, window: u16) {
        push_syn(TcpSynInfo {
            parse_ok: true,
            src_ip: Some(ip.into()),
            src_port: Some(sport),
            dst_port: Some(28780),
            ip_ttl: Some(64),
            tcp_window: Some(window),
            tcp_mss: Some(1460),
            tcp_wscale: Some(7),
            tcp_sack_ok: true,
            tcp_timestamp: true,
            tcp_flags: Some(0x02),
            tcp_flags_str: Some("S".into()),
            option_order_str: Some("mss,sack,ts,ws".into()),
            p0f_sig: Some(format!("lab_p0f_{sport}")),
            ..Default::default()
        });
    }

    #[test]
    fn g1_same_ip_different_ports_no_cross_join() {
        let _g = test_lock();
        test_clear_store();
        push_syn_ports("203.0.113.10", 40001, 11111);
        push_syn_ports("203.0.113.10", 40002, 22222);

        // Request on port 40001 must get window 11111, not the other tenant.
        let mut fields = Map::new();
        enrich_protocol_fields_scoped(
            &mut fields,
            &JoinScope {
                client_ip: Some("203.0.113.10".into()),
                src_port: Some(40001),
                dst_port: Some(28780),
                conn_token: None,
            },
        );
        assert_eq!(fields.get("tcp_syn_window").and_then(|v| v.as_u64()), Some(11111));
        assert_eq!(
            fields.get("side_join_strength").and_then(|v| v.as_str()),
            Some("ip_conn_ttl_strong")
        );
        assert_eq!(
            fields
                .get("side_join_commercial_merge_allowed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            fields
                .get("side_join_soft_hard_link_allowed")
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        let mut fields_b = Map::new();
        enrich_protocol_fields_scoped(
            &mut fields_b,
            &JoinScope {
                client_ip: Some("203.0.113.10".into()),
                src_port: Some(40002),
                ..Default::default()
            },
        );
        assert_eq!(
            fields_b.get("tcp_syn_window").and_then(|v| v.as_u64()),
            Some(22222)
        );
        assert!(fields_b
            .get("side_join_is_strong")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    #[test]
    fn g1_weak_ip_only_never_commercial_merge() {
        let _g = test_lock();
        test_clear_store();
        push_syn_ports("198.51.100.9", 50001, 33333);
        let mut fields = Map::new();
        // No request port → IP-only weak join still enriches but never commercial merge.
        enrich_protocol_fields(&mut fields, Some("198.51.100.9"));
        assert_eq!(fields.get("tcp_syn_present").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            fields.get("side_join_strength").and_then(|v| v.as_str()),
            Some("ip_ttl_window_weak")
        );
        assert_eq!(
            fields
                .get("side_join_commercial_merge_allowed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            fields
                .get("side_join_soft_hard_link_allowed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        let policy = fields.get("side_join_policy").cloned().unwrap_or(json!({}));
        assert_eq!(
            policy
                .get("commercial_device_hard_merge")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn g1_port_mismatch_does_not_attach_wrong_sample() {
        let _g = test_lock();
        test_clear_store();
        push_syn_ports("203.0.113.50", 41000, 44444);
        let mut fields = Map::new();
        enrich_protocol_fields_scoped(
            &mut fields,
            &JoinScope {
                client_ip: Some("203.0.113.50".into()),
                src_port: Some(41999), // different port
                ..Default::default()
            },
        );
        // Explicit mismatch → no SYN attach
        assert!(
            fields.get("tcp_syn_present").is_none()
                || fields.get("tcp_syn_present") == Some(&json!(false))
        );
        assert_eq!(
            fields.get("side_join_strength").and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            fields
                .get("side_join_commercial_merge_allowed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn g1_strong_quic_enriches_ja4() {
        let _g = test_lock();
        test_clear_store();
        push_quic(QuicInitialInfo {
            parse_ok: true,
            src_ip: Some("192.0.2.77".into()),
            src_port: Some(4433),
            tls_ja4: Some("t13d1516h2_8daaf6152771_b0d3e53935d8".into()),
            quic_fp: Some("lab_fp_strong".into()),
            aead_ok: true,
            ..Default::default()
        });
        let mut fields = Map::new();
        enrich_protocol_fields_scoped(
            &mut fields,
            &JoinScope {
                client_ip: Some("192.0.2.77".into()),
                src_port: Some(4433),
                ..Default::default()
            },
        );
        assert_eq!(fields.get("quic_listen_present").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            fields.get("quic_tls_ja4").and_then(|v| v.as_str()),
            Some("t13d1516h2_8daaf6152771_b0d3e53935d8")
        );
        assert!(fields
            .get("side_join_is_strong")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        // Even strong side-channel never hard-merges commercial device id.
        assert_eq!(
            fields
                .get("side_join_commercial_merge_allowed")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    fn push_h3_ports(ip: &str, sport: u16, pseudo: &str, settings_fp: &str) {
        push_h3_app(H3AppInfo {
            present: true,
            src_ip: ip.into(),
            src_port: sport,
            alpn: "h3".into(),
            method: "POST".into(),
            path: "/v1/gateway/early".into(),
            pseudo_order: pseudo.into(),
            settings_fp: settings_fp.into(),
            rtt_ms: Some(12.0),
            note: "test".into(),
            ..Default::default()
        });
    }

    #[test]
    fn g1_h3_same_ip_different_ports_inject_no_cross() {
        let _g = test_lock();
        test_clear_store();
        push_h3_ports("203.0.113.88", 50001, ":method,:path,a", "fp_a_port50001");
        push_h3_ports("203.0.113.88", 50002, ":method,:path,b", "fp_b_port50002");

        // Port-scoped inject must not pick the other NAT peer.
        let mut hdr_a: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        inject_h3_app_headers_scoped(
            &mut hdr_a,
            &JoinScope {
                client_ip: Some("203.0.113.88".into()),
                src_port: Some(50001),
                ..Default::default()
            },
        );
        assert_eq!(hdr_a.get("x-gr-h3-settings-fp").map(|s| s.as_str()), Some("fp_a_port50001"));
        assert_eq!(
            hdr_a.get("x-gr-h3-pseudo-order").map(|s| s.as_str()),
            Some(":method,:path,a")
        );
        assert!(
            hdr_a
                .get("x-gr-h3-join-strength")
                .map(|s| s.contains("strong"))
                .unwrap_or(false),
            "expected strong join, got {:?}",
            hdr_a.get("x-gr-h3-join-strength")
        );

        let mut hdr_b: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        inject_h3_app_headers_scoped(
            &mut hdr_b,
            &JoinScope {
                client_ip: Some("203.0.113.88".into()),
                src_port: Some(50002),
                ..Default::default()
            },
        );
        assert_eq!(hdr_b.get("x-gr-h3-settings-fp").map(|s| s.as_str()), Some("fp_b_port50002"));
    }

    #[test]
    fn g1_h3_from_info_never_cross_and_handlers_no_stomp() {
        let _g = test_lock();
        test_clear_store();
        // Two samples same IP — from_info uses **this** sample only.
        let mine = H3AppInfo {
            present: true,
            src_ip: "198.51.100.20".into(),
            src_port: 60001,
            alpn: "h3".into(),
            pseudo_order: ":method,:authority,:path".into(),
            settings_fp: "mine_fp_60001".into(),
            rtt_ms: Some(8.0),
            note: "this_conn".into(),
            ..Default::default()
        };
        push_h3_app(H3AppInfo {
            present: true,
            src_ip: "198.51.100.20".into(),
            src_port: 60002,
            alpn: "h3".into(),
            pseudo_order: ":method,:path,OTHER".into(),
            settings_fp: "other_fp_60002".into(),
            ..Default::default()
        });
        push_h3_app(mine.clone());

        let mut hdr = std::collections::HashMap::new();
        inject_h3_app_headers_from_info(&mut hdr, &mine);
        assert_eq!(hdr.get("x-gr-h3-settings-fp").map(|s| s.as_str()), Some("mine_fp_60001"));
        assert_eq!(
            hdr.get("x-gr-side-join-strength").map(|s| s.as_str()),
            Some("ip_conn_ttl_strong")
        );

        // Simulate handlers path: port-scoped enrich first, then H3 header fill must not stomp.
        let mut fields = Map::new();
        enrich_protocol_fields_scoped(
            &mut fields,
            &JoinScope {
                client_ip: Some("198.51.100.20".into()),
                src_port: Some(60001),
                ..Default::default()
            },
        );
        assert_eq!(
            fields.get("h3_settings_fp").and_then(|v| v.as_str()),
            Some("mine_fp_60001")
        );
        // Simulate handlers: only fill missing — never overwrite scoped enrich.
        if !fields.contains_key("h3_settings_fp") {
            fields.insert("h3_settings_fp".into(), json!("other_fp_60002"));
        }
        assert_eq!(
            fields.get("h3_settings_fp").and_then(|v| v.as_str()),
            Some("mine_fp_60001"),
            "strong enrich must not be stomped by later weak header fill"
        );
        // Honesty re-stamp after H3 fields
        gr_probe_core::annotate_protocol_export_honesty(&mut fields);
        assert_eq!(
            fields
                .get("h3_pseudo_order__diagnostic_only")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            fields
                .get("h3_settings_fp__diagnostic_only")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn g1_h3_port_mismatch_no_inject() {
        let _g = test_lock();
        test_clear_store();
        push_h3_ports("203.0.113.99", 51000, ":method,:path", "fp_only");
        let mut hdr = std::collections::HashMap::new();
        inject_h3_app_headers_scoped(
            &mut hdr,
            &JoinScope {
                client_ip: Some("203.0.113.99".into()),
                src_port: Some(51999),
                ..Default::default()
            },
        );
        assert!(
            hdr.get("x-gr-h3-settings-fp").is_none(),
            "port mismatch must not inject wrong peer: {:?}",
            hdr
        );
    }
}
