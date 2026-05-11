use super::{Probe, ProbeResult};
use std::collections::HashMap;

pub struct MetricProbe<'a> {
    pub metric: &'a str,
    pub path: Option<&'a str>,
}

impl<'a> Probe for MetricProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let value = match self.metric {
            "cpu.count" => get_cpu_count(),
            "cpu.load_1m" => get_load_avg(0),
            "cpu.load_5m" => get_load_avg(1),
            "cpu.load_15m" => get_load_avg(2),
            "memory.total_mb" => get_memory("MemTotal"),
            "memory.available_mb" => get_memory("MemAvailable"),
            "memory.used_mb" => {
                let total = get_memory("MemTotal");
                let avail = get_memory("MemAvailable");
                match (total, avail) {
                    (Some(t), Some(a)) => Some(t - a),
                    _ => None,
                }
            }
            "disk.total_gb" => get_disk_stat(self.path.unwrap_or("."), DiskStat::Total),
            "disk.available_gb" => get_disk_stat(self.path.unwrap_or("."), DiskStat::Available),
            "disk.used_gb" => get_disk_stat(self.path.unwrap_or("."), DiskStat::Used),
            "swap.total_mb" => get_memory("SwapTotal"),
            "swap.used_mb" => {
                let total = get_memory("SwapTotal");
                let free = get_memory("SwapFree");
                match (total, free) {
                    (Some(t), Some(f)) => Some(t - f),
                    _ => None,
                }
            }
            _ => None,
        };

        match value {
            Some(v) => {
                let metric_var = format!("METRIC_{}", self.metric.replace('.', "_").to_uppercase());
                let mut variables = HashMap::new();
                variables.insert(metric_var, format!("{v:.1}"));

                ProbeResult {
                    success: true,
                    hash: blake3::hash(format!("{v}").as_bytes()).to_hex().to_string(),
                    variables,
                    error: None,
                }
            }
            None => ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!("metric '{}' not available on this platform", self.metric)),
            },
        }
    }
}

#[cfg(target_os = "linux")]
fn get_load_avg(index: usize) -> Option<f64> {
    let content = std::fs::read_to_string("/proc/loadavg").ok()?;
    content.split_whitespace().nth(index)?.parse::<f64>().ok()
}

#[cfg(not(target_os = "linux"))]
fn get_load_avg(_index: usize) -> Option<f64> {
    None
}

fn get_cpu_count() -> Option<f64> {
    std::thread::available_parallelism()
        .ok()
        .map(|n| n.get() as f64)
}

#[cfg(target_os = "linux")]
fn get_memory(field: &str) -> Option<f64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if line.starts_with(field) {
            // Format: "MemTotal:       16384000 kB"
            let kb = line.split_whitespace().nth(1)?.parse::<f64>().ok()?;
            return Some(kb / 1024.0); // Convert to MB
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn get_memory(_field: &str) -> Option<f64> {
    None
}

enum DiskStat {
    Total,
    Available,
    Used,
}

fn get_disk_stat(path: &str, stat: DiskStat) -> Option<f64> {
    let c_path = std::ffi::CString::new(path).ok()?;
    let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut buf) };
    if ret != 0 {
        return None;
    }

    let block_size = buf.f_frsize as f64;
    let gb = 1024.0 * 1024.0 * 1024.0;

    match stat {
        DiskStat::Total => Some(buf.f_blocks as f64 * block_size / gb),
        DiskStat::Available => Some(buf.f_bavail as f64 * block_size / gb),
        DiskStat::Used => {
            Some((buf.f_blocks - buf.f_bfree) as f64 * block_size / gb)
        }
    }
}
