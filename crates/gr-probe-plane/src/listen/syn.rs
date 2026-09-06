//! TCP SYN p0f-style capture (optional CAP_NET_RAW).

use crate::listen::side_store::push_syn;
use gr_probe_net::parse_tcp_syn_packet;
use log::{info, warn};
use std::thread;

/// Spawn background SYN sniffer. Requires CAP_NET_RAW / root for AF_PACKET.
pub fn spawn_syn_listener(filter_ports: Vec<u16>) {
    thread::Builder::new()
        .name("gr-syn-listen".into())
        .spawn(move || {
            if let Err(e) = run_af_packet(filter_ports) {
                warn!(
                    "TCP SYN listen disabled ({e}). Run with CAP_NET_RAW for p0f-style capture."
                );
            }
        })
        .ok();
}

#[cfg(target_os = "linux")]
fn run_af_packet(filter_ports: Vec<u16>) -> std::io::Result<()> {
    let sock = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW,
            (libc::ETH_P_ALL as u16).to_be() as i32,
        )
    };
    if sock < 0 {
        return Err(std::io::Error::last_os_error());
    }
    info!(
        "TCP SYN AF_PACKET listener started (filter ports={filter_ports:?})"
    );
    let mut buf = vec![0u8; 65536];
    loop {
        let n = unsafe {
            libc::recvfrom(
                sock,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if n <= 0 {
            continue;
        }
        let frame = &buf[..n as usize];
        // Ethernet II: skip 14 if looks like ethertype IPv4/IPv6
        let payload = if frame.len() > 14 && (frame[12] == 0x08 || frame[12] == 0x86) {
            &frame[14..]
        } else {
            frame
        };
        if let Some(info) = parse_tcp_syn_packet(payload) {
            if info.parse_ok {
                if let Some(dp) = info.dst_port {
                    if !filter_ports.is_empty() && !filter_ports.contains(&dp) {
                        continue;
                    }
                }
                // only SYN without ACK
                if info.tcp_flags.map(|f| f & 0x12 == 0x02).unwrap_or(false) {
                    push_syn(info);
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn run_af_packet(_filter_ports: Vec<u16>) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "AF_PACKET SYN listen only on Linux",
    ))
}
