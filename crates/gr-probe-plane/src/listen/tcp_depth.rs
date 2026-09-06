//! TCP depth: TCP_SAVED_SYN (no CAP_NET_RAW) + TCP_INFO + sockopts (P-V3).
//!
//! Pingora enables TCP_SAVE_SYN on listeners; we read SAVED_SYN from the accepted
//! connection fd and push into side_store for gateway join.

use crate::listen::side_store;
use gr_probe_net::{parse_tcp_syn_packet, TcpSynInfo};
use log::debug;
use pingora_proxy::Session;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Capture TCP depth for this session → headers + side_store SYN.
pub fn inject_tcp_depth(session: &Session, headers: &mut HashMap<String, String>) {
    #[cfg(target_os = "linux")]
    {
        let Some(sock) = session
            .digest()
            .and_then(|d| d.socket_digest.as_ref().map(|a| a.clone()))
        else {
            return;
        };
        let fd = sock.raw_fd();
        if let Some(mut syn) = try_read_saved_syn(fd) {
            if syn.src_ip.is_none() {
                if let Some(addr) = session.client_addr().and_then(|a| a.as_inet()) {
                    syn.src_ip = Some(addr.ip().to_string());
                    syn.src_port = Some(addr.port());
                }
            }
            headers.insert("x-gr-tcp-saved-syn".into(), "1".into());
            if let Some(ref sig) = syn.option_order_str {
                headers.insert("x-gr-tcp-syn-opts".into(), sig.clone());
            }
            debug!(
                "TCP_SAVED_SYN ok mss={:?} win={:?} opts={:?}",
                syn.tcp_mss, syn.tcp_window, syn.option_order_str
            );
            side_store::push_syn(syn);
        }
        if let Some(info) = sock.tcp_info() {
            inject_tcp_info_headers(&info, headers);
        }
        inject_sockopt_headers(fd, headers);
    }
    let _ = (session, headers);
}

/// Merge TCP depth header fields into gateway fields map.
pub fn apply_tcp_depth_to_fields(fields: &mut Map<String, Value>, headers: &HashMap<String, String>) {
    if headers.get("x-gr-tcp-info").map(|s| s == "1").unwrap_or(false)
        || headers.contains_key("x-gr-tcp-rtt-us")
    {
        fields.insert("tcp_info_available".into(), json!(true));
    }
    if let Some(v) = headers.get("x-gr-tcp-rtt-us").and_then(|s| s.parse::<u64>().ok()) {
        fields.insert("tcp_info_rtt_us".into(), json!(v));
        // CF/xsrc parity name
        fields.insert("client_tcp_rtt_us".into(), json!(v));
        fields.insert("client_tcp_rtt".into(), json!(v));
        // iss/45 B6: JA4L partial — RTT + optional min_rtt/retrans (not full FoxIO JA4L)
        let ms = (v as f64) / 1000.0;
        let bucket = if ms < 20.0 {
            "lt20ms"
        } else if ms < 50.0 {
            "20_50ms"
        } else if ms < 100.0 {
            "50_100ms"
        } else if ms < 200.0 {
            "100_200ms"
        } else {
            "gte200ms"
        };
        fields.insert("ja4l_lite".into(), json!(format!("t_{bucket}")));
        fields.insert("ja4l_lite_rtt_ms".into(), json!((ms * 10.0).round() / 10.0));
        fields.insert("ja4l_lite_role".into(), json!("network_timing_conf_only"));
        let min_rtt_us = headers
            .get("x-gr-tcp-min-rtt-us")
            .and_then(|s| s.parse::<u64>().ok());
        let retrans = headers
            .get("x-gr-tcp-total-retrans")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let min_ms = min_rtt_us.map(|u| (u as f64) / 1000.0).unwrap_or(ms);
        let min_bucket = if min_ms < 20.0 {
            "lt20"
        } else if min_ms < 50.0 {
            "20_50"
        } else if min_ms < 100.0 {
            "50_100"
        } else {
            "gte100"
        };
        let retx_flag = if retrans == 0 {
            "r0"
        } else if retrans < 3 {
            "r1"
        } else {
            "rN"
        };
        // Composite partial JA4L: distance-like timing fingerprint
        let ja4l = format!("q{bucket}_{min_bucket}_{retx_flag}");
        fields.insert("ja4l".into(), json!(ja4l));
        fields.insert("ja4l_role".into(), json!("tcp_timing_conf_only"));
        fields.insert("ja4l__commercial_mint".into(), json!(false));
        fields.insert("ja4l__full_foxio".into(), json!(false));
        fields.insert("ja4l_partial".into(), json!(true));
    }
    if let Some(v) = headers
        .get("x-gr-tcp-min-rtt-us")
        .and_then(|s| s.parse::<u64>().ok())
    {
        fields.insert("tcp_info_min_rtt_us".into(), json!(v));
    }
    if let Some(v) = headers
        .get("x-gr-tcp-snd-cwnd")
        .and_then(|s| s.parse::<u64>().ok())
    {
        fields.insert("tcp_info_snd_cwnd".into(), json!(v));
    }
    if let Some(v) = headers
        .get("x-gr-tcp-total-retrans")
        .and_then(|s| s.parse::<u64>().ok())
    {
        fields.insert("tcp_info_total_retrans".into(), json!(v));
    }
    if let Some(v) = headers
        .get("x-gr-tcp-mss")
        .and_then(|s| s.parse::<u64>().ok())
    {
        fields.insert("tcp_info_snd_mss".into(), json!(v));
    }
    if let Some(v) = headers.get("x-gr-tcp-congestion") {
        fields.insert("tcp_congestion".into(), json!(v));
    }
    if headers.get("x-gr-tcp-saved-syn").map(|s| s == "1").unwrap_or(false) {
        fields.insert("tcp_saved_syn".into(), json!(true));
    }
    if let Some(opts) = headers.get("x-gr-tcp-syn-opts") {
        if !opts.is_empty() {
            fields.insert("tcp_syn_option_order".into(), json!(opts));
        }
    }
}

#[cfg(target_os = "linux")]
fn try_read_saved_syn(fd: std::os::unix::io::RawFd) -> Option<TcpSynInfo> {
    const TCP_SAVED_SYN: libc::c_int = 28;
    let mut buf = vec![0u8; 512];
    let mut len = buf.len() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            TCP_SAVED_SYN,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let n = (len as usize).min(buf.len());
    let raw = &buf[..n];
    let mut info = parse_tcp_syn_packet(raw)?;
    info.notes
        .push(format!("source=TCP_SAVED_SYN len={n}"));
    Some(info)
}

#[cfg(target_os = "linux")]
fn inject_tcp_info_headers(
    info: &pingora_core::protocols::l4::ext::TCP_INFO,
    headers: &mut HashMap<String, String>,
) {
    headers.insert("x-gr-tcp-info".into(), "1".into());
    if info.tcpi_rtt > 0 {
        headers.insert("x-gr-tcp-rtt-us".into(), info.tcpi_rtt.to_string());
    }
    if info.tcpi_min_rtt > 0 {
        headers.insert(
            "x-gr-tcp-min-rtt-us".into(),
            info.tcpi_min_rtt.to_string(),
        );
    }
    if info.tcpi_snd_cwnd > 0 {
        headers.insert(
            "x-gr-tcp-snd-cwnd".into(),
            info.tcpi_snd_cwnd.to_string(),
        );
    }
    if info.tcpi_total_retrans > 0 {
        headers.insert(
            "x-gr-tcp-total-retrans".into(),
            info.tcpi_total_retrans.to_string(),
        );
    }
    if info.tcpi_snd_mss > 0 {
        headers.insert("x-gr-tcp-mss".into(), info.tcpi_snd_mss.to_string());
    }
    if info.tcpi_bytes_acked > 0 {
        headers.insert(
            "x-gr-tcp-bytes-acked".into(),
            info.tcpi_bytes_acked.to_string(),
        );
    }
}

#[cfg(target_os = "linux")]
fn inject_sockopt_headers(fd: i32, headers: &mut HashMap<String, String>) {
    if let Ok(v) = getsockopt_i32(fd, libc::IPPROTO_TCP, libc::TCP_MAXSEG) {
        if v > 0 {
            headers
                .entry("x-gr-tcp-mss".into())
                .or_insert_with(|| v.to_string());
        }
    }
    if let Ok(s) = getsockopt_str(fd, libc::IPPROTO_TCP, libc::TCP_CONGESTION, 32) {
        if !s.is_empty() {
            headers.insert("x-gr-tcp-congestion".into(), s);
        }
    }
}

#[cfg(target_os = "linux")]
fn getsockopt_i32(fd: i32, level: i32, opt: i32) -> std::io::Result<i32> {
    let mut v: i32 = 0;
    let mut len = std::mem::size_of::<i32>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            level,
            opt,
            &mut v as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(v)
}

#[cfg(target_os = "linux")]
fn getsockopt_str(fd: i32, level: i32, opt: i32, max: usize) -> std::io::Result<String> {
    let mut buf = vec![0u8; max];
    let mut len = buf.len() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            level,
            opt,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let n = (len as usize).min(buf.len());
    let end = buf[..n].iter().position(|&b| b == 0).unwrap_or(n);
    Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
}
