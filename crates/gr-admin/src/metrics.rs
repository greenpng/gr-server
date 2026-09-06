//! Host resource metrics for admin dashboard.

use serde::Serialize;
use sysinfo::{Disks, Networks, System};

#[derive(Debug, Clone, Serialize)]
pub struct SystemMetrics {
    pub cpu_pct: f32,
    pub mem_total_bytes: u64,
    pub mem_used_bytes: u64,
    pub mem_pct: f32,
    pub load_avg_one: f64,
    pub disks: Vec<DiskMetric>,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskMetric {
    pub name: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_pct: f32,
}

pub fn sample() -> SystemMetrics {
    let mut sys = System::new_all();
    sys.refresh_all();
    // brief second sample for cpu
    std::thread::sleep(std::time::Duration::from_millis(100));
    sys.refresh_cpu();
    let cpu_pct = sys.global_cpu_info().cpu_usage();
    let mem_total = sys.total_memory();
    let mem_used = sys.used_memory();
    let mem_pct = if mem_total > 0 {
        (mem_used as f32 / mem_total as f32) * 100.0
    } else {
        0.0
    };
    let load = System::load_average();
    let disks_list = Disks::new_with_refreshed_list();
    let mut disks = Vec::new();
    for d in disks_list.list() {
        let total = d.total_space();
        let avail = d.available_space();
        let used_pct = if total > 0 {
            ((total - avail) as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        disks.push(DiskMetric {
            name: d.name().to_string_lossy().to_string(),
            total_bytes: total,
            available_bytes: avail,
            used_pct,
        });
    }
    let nets = Networks::new_with_refreshed_list();
    let mut rx = 0u64;
    let mut tx = 0u64;
    for (_name, data) in nets.list() {
        rx = rx.saturating_add(data.total_received());
        tx = tx.saturating_add(data.total_transmitted());
    }
    SystemMetrics {
        cpu_pct,
        mem_total_bytes: mem_total,
        mem_used_bytes: mem_used,
        mem_pct,
        load_avg_one: load.one,
        disks,
        net_rx_bytes: rx,
        net_tx_bytes: tx,
    }
}
