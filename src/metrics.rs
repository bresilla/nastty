//! Metrics collection owned by `nastty serve`.
//!
//! The upstream project runs these collectors in a second daemon. Nastty keeps
//! them in-process so operators only need the single `nastty` executable.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nasty_common::metrics_types::{
    CategoryCount, CpuStats, DiskHealth, DiskIoStats, IoSample, KernelError, KernelErrorSummary,
    MemoryStats, NetIfStats, ResourceHistory, SystemStats,
};
use tokio::sync::RwLock;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy)]
struct Sample {
    ts: i64,
    in_rate: f64,
    out_rate: f64,
}

#[derive(Default)]
struct History {
    series: HashMap<(String, String), VecDeque<Sample>>,
}

/// Live metrics and bounded in-memory history collected by `nastty serve`.
pub struct MetricsService {
    stats: RwLock<SystemStats>,
    disks: RwLock<Vec<DiskHealth>>,
    kernel_errors: RwLock<KernelErrorSummary>,
    history: Mutex<History>,
}

impl MetricsService {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            stats: RwLock::new(system_stats()),
            disks: RwLock::new(Vec::new()),
            kernel_errors: RwLock::new(KernelErrorSummary::default()),
            history: Mutex::new(History::default()),
        })
    }

    /// Start the built-in sampler. This task lives for the server lifetime.
    pub fn start(self: &Arc<Self>) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut previous = service.stats.read().await.clone();
            let mut previous_at = Instant::now();
            let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await;

            loop {
                tick.tick().await;
                let collected = tokio::task::spawn_blocking(system_stats).await;
                let current = match collected {
                    Ok(stats) => stats,
                    Err(error) => {
                        tracing::warn!("metrics collector worker failed: {error}");
                        continue;
                    }
                };
                let now = Instant::now();
                let elapsed = now.duration_since(previous_at).as_secs_f64();
                if elapsed > 0.0 {
                    service.record_rates(&previous, &current, elapsed);
                }
                *service.stats.write().await = current.clone();
                previous = current;
                previous_at = now;
            }
        });

        let service = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                match tokio::task::spawn_blocking(disk_health).await {
                    Ok(disks) => *service.disks.write().await = disks,
                    Err(error) => tracing::warn!("SMART collector worker failed: {error}"),
                }
            }
        });

        let service = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                match tokio::task::spawn_blocking(kernel_errors).await {
                    Ok(errors) => *service.kernel_errors.write().await = errors,
                    Err(error) => tracing::warn!("kernel-error collector worker failed: {error}"),
                }
            }
        });
    }

    pub async fn stats(&self) -> SystemStats {
        self.stats.read().await.clone()
    }

    pub async fn disks(&self) -> Vec<DiskHealth> {
        self.disks.read().await.clone()
    }

    pub async fn kernel_errors(&self) -> KernelErrorSummary {
        self.kernel_errors.read().await.clone()
    }

    pub fn history(
        &self,
        kind: &str,
        name: Option<&str>,
        range: &str,
        offset_ms: i64,
    ) -> Vec<ResourceHistory> {
        let (duration_ms, bucket_ms) = history_window(range);
        let until = now_ms().saturating_sub(offset_ms.max(0));
        let since = until.saturating_sub(duration_ms);
        let history = self.history.lock().expect("metrics history mutex poisoned");
        let mut names = history
            .series
            .keys()
            .filter(|(series_kind, series_name)| {
                series_kind == kind && name.is_none_or(|wanted| wanted == series_name)
            })
            .map(|(_, series_name)| series_name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();

        names
            .into_iter()
            .map(|series_name| {
                let samples = history
                    .series
                    .get(&(kind.to_string(), series_name.clone()))
                    .map(|series| window_samples(series, since, until, bucket_ms))
                    .unwrap_or_default();
                ResourceHistory {
                    name: series_name,
                    samples,
                }
            })
            .collect()
    }

    pub async fn prometheus(&self) -> String {
        let stats = self.stats().await;
        let disks = self.disks().await;
        let kernel_errors = self.kernel_errors().await;
        let mut out = String::with_capacity(4 * 1_024);
        metric(&mut out, "nastty_cpu_count", stats.cpu.count as f64);
        metric(&mut out, "nastty_cpu_load_1m", stats.cpu.load_1);
        metric(&mut out, "nastty_cpu_load_5m", stats.cpu.load_5);
        metric(&mut out, "nastty_cpu_load_15m", stats.cpu.load_15);
        metric(
            &mut out,
            "nastty_memory_total_bytes",
            stats.memory.total_bytes as f64,
        );
        metric(
            &mut out,
            "nastty_memory_used_bytes",
            stats.memory.used_bytes as f64,
        );
        metric(
            &mut out,
            "nastty_memory_available_bytes",
            stats.memory.available_bytes as f64,
        );
        for interface in &stats.network {
            labelled_metric(
                &mut out,
                "nastty_net_rx_bytes_total",
                "interface",
                &interface.name,
                interface.rx_bytes as f64,
            );
            labelled_metric(
                &mut out,
                "nastty_net_tx_bytes_total",
                "interface",
                &interface.name,
                interface.tx_bytes as f64,
            );
        }
        for disk in &stats.disk_io {
            labelled_metric(
                &mut out,
                "nastty_disk_read_bytes_total",
                "device",
                &disk.name,
                disk.read_bytes as f64,
            );
            labelled_metric(
                &mut out,
                "nastty_disk_write_bytes_total",
                "device",
                &disk.name,
                disk.write_bytes as f64,
            );
        }
        for disk in &disks {
            labelled_metric(
                &mut out,
                "nastty_disk_smart_healthy",
                "device",
                &disk.device,
                if disk.health_passed { 1.0 } else { 0.0 },
            );
            if let Some(temperature) = disk.temperature_c {
                labelled_metric(
                    &mut out,
                    "nastty_disk_temperature_celsius",
                    "device",
                    &disk.device,
                    temperature as f64,
                );
            }
        }
        metric(
            &mut out,
            "nastty_kernel_errors_total",
            kernel_errors.total_count as f64,
        );
        for category in &kernel_errors.by_category {
            labelled_metric(
                &mut out,
                "nastty_kernel_errors_by_category",
                "category",
                &category.category,
                category.count as f64,
            );
        }
        out
    }

    fn record_rates(&self, previous: &SystemStats, current: &SystemStats, elapsed: f64) {
        for interface in &current.network {
            if let Some(old) = previous
                .network
                .iter()
                .find(|item| item.name == interface.name)
            {
                self.record(
                    "net",
                    &interface.name,
                    interface.rx_bytes.saturating_sub(old.rx_bytes) as f64 / elapsed,
                    interface.tx_bytes.saturating_sub(old.tx_bytes) as f64 / elapsed,
                );
            }
        }
        for disk in &current.disk_io {
            if let Some(old) = previous.disk_io.iter().find(|item| item.name == disk.name) {
                self.record(
                    "disk",
                    &disk.name,
                    disk.read_bytes.saturating_sub(old.read_bytes) as f64 / elapsed,
                    disk.write_bytes.saturating_sub(old.write_bytes) as f64 / elapsed,
                );
            }
        }
        let cores = current.cpu.count.max(1) as f64;
        self.record(
            "cpu",
            "cpu",
            (current.cpu.load_1 / cores * 100.0).clamp(0.0, 100.0),
            0.0,
        );
        let memory = if current.memory.total_bytes == 0 {
            0.0
        } else {
            current.memory.used_bytes as f64 / current.memory.total_bytes as f64 * 100.0
        };
        self.record("mem", "mem", memory, 0.0);
    }

    fn record(&self, kind: &str, name: &str, in_rate: f64, out_rate: f64) {
        let now = now_ms();
        let cutoff = now.saturating_sub(RETENTION_MS);
        let mut history = self.history.lock().expect("metrics history mutex poisoned");
        let series = history
            .series
            .entry((kind.to_string(), name.to_string()))
            .or_default();
        series.push_back(Sample {
            ts: now,
            in_rate,
            out_rate,
        });
        while series.front().is_some_and(|sample| sample.ts < cutoff) {
            series.pop_front();
        }
    }
}

fn system_stats() -> SystemStats {
    SystemStats {
        cpu: cpu_stats(),
        memory: memory_stats(),
        network: network_stats(),
        disk_io: disk_io_stats(),
    }
}

fn cpu_stats() -> CpuStats {
    let count = std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1);
    let loads = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|contents| {
            let mut values = contents.split_whitespace();
            Some((
                values.next()?.parse().ok()?,
                values.next()?.parse().ok()?,
                values.next()?.parse().ok()?,
            ))
        })
        .unwrap_or((0.0, 0.0, 0.0));
    CpuStats {
        count,
        load_1: loads.0,
        load_5: loads.1,
        load_15: loads.2,
        temp_c: cpu_temperature(),
        freq_mhz: cpu_frequency(),
        governor: std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }
}

fn cpu_temperature() -> Option<i32> {
    let entries = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = std::fs::read_to_string(path.join("name")).unwrap_or_default();
        if matches!(name.trim(), "coretemp" | "k10temp" | "zenpower")
            && let Ok(value) = std::fs::read_to_string(path.join("temp1_input"))
            && let Ok(millidegrees) = value.trim().parse::<i64>()
        {
            return Some((millidegrees / 1_000) as i32);
        }
    }
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
        .map(|value| (value / 1_000) as i32)
}

fn cpu_frequency() -> Option<u32> {
    let entries = std::fs::read_dir("/sys/devices/system/cpu").ok()?;
    let mut total = 0u64;
    let mut count = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name
            .strip_prefix("cpu")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        if let Ok(value) = std::fs::read_to_string(entry.path().join("cpufreq/scaling_cur_freq"))
            && let Ok(khz) = value.trim().parse::<u64>()
        {
            total += khz;
            count += 1;
        }
    }
    (count > 0).then(|| (total / count / 1_000) as u32)
}

fn memory_stats() -> MemoryStats {
    let mut values = HashMap::new();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    for line in meminfo.lines() {
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else { continue };
        let value = fields
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            * 1_024;
        values.insert(key.trim_end_matches(':'), value);
    }
    let total = values.get("MemTotal").copied().unwrap_or(0);
    let available = values.get("MemAvailable").copied().unwrap_or(0);
    let swap_total = values.get("SwapTotal").copied().unwrap_or(0);
    let swap_free = values.get("SwapFree").copied().unwrap_or(0);
    MemoryStats {
        total_bytes: total,
        used_bytes: total.saturating_sub(available),
        available_bytes: available,
        swap_total_bytes: swap_total,
        swap_used_bytes: swap_total.saturating_sub(swap_free),
    }
}

fn network_stats() -> Vec<NetIfStats> {
    let mut interfaces = Vec::new();
    for line in std::fs::read_to_string("/proc/net/dev")
        .unwrap_or_default()
        .lines()
        .skip(2)
    {
        let Some((name, counters)) = line.trim().split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" {
            continue;
        }
        let values = counters
            .split_whitespace()
            .filter_map(|value| value.parse::<u64>().ok())
            .collect::<Vec<_>>();
        if values.len() < 10 {
            continue;
        }
        interfaces.push(NetIfStats {
            name: name.to_string(),
            rx_bytes: values[0],
            tx_bytes: values[8],
            rx_packets: values[1],
            tx_packets: values[9],
            speed_mbps: read_u32(&format!("/sys/class/net/{name}/speed")),
            up: std::fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
                .is_ok_and(|value| value.trim() == "up"),
            addresses: Vec::new(),
        });
    }
    interfaces
}

fn disk_io_stats() -> Vec<DiskIoStats> {
    let mut disks = Vec::new();
    for line in std::fs::read_to_string("/proc/diskstats")
        .unwrap_or_default()
        .lines()
    {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 14 {
            continue;
        }
        let name = fields[2];
        let whole_disk = (name.starts_with("sd") && name.len() == 3)
            || (name.starts_with("vd") && name.len() == 3)
            || (name.starts_with("nvme") && name.contains('n') && !name.contains('p'));
        if !whole_disk {
            continue;
        }
        let number = |index: usize| fields[index].parse::<u64>().unwrap_or(0);
        disks.push(DiskIoStats {
            name: name.to_string(),
            read_bytes: number(5) * 512,
            write_bytes: number(9) * 512,
            read_ios: number(3),
            write_ios: number(7),
            io_in_progress: number(11),
        });
    }
    disks.sort_by(|left, right| left.name.cmp(&right.name));
    disks
}

/// Collect the generic SMART fields used by nastty's device table and alerts.
/// Vendor-specific ATA/NVMe/SAS detail remains optional in the shared schema.
fn disk_health() -> Vec<DiskHealth> {
    if !command_exists("smartctl") {
        return Vec::new();
    }
    disk_io_stats()
        .into_iter()
        .filter_map(|disk| {
            let device = format!("/dev/{}", disk.name);
            let output = std::process::Command::new("smartctl")
                .args(["-a", "-j", &device])
                .output()
                .ok()?;
            // smartctl uses non-zero bits for health findings while still
            // returning a valid JSON report, so parse stdout regardless.
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            let passed = value
                .pointer("/smart_status/passed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let rotation = value.get("rotation_rate").and_then(|value| value.as_u64());
            Some(DiskHealth {
                device,
                transport: None,
                ata_port: None,
                controller_pci: None,
                controller_name: None,
                pcie_link: None,
                model: json_string(&value, "model_name"),
                serial: json_string(&value, "serial_number"),
                firmware: json_string(&value, "firmware_version"),
                capacity_bytes: value
                    .pointer("/user_capacity/bytes")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                temperature_c: value
                    .pointer("/temperature/current")
                    .and_then(|value| value.as_i64())
                    .map(|value| value as i32),
                power_on_hours: value
                    .pointer("/power_on_time/hours")
                    .and_then(|value| value.as_u64()),
                health_passed: passed,
                smart_status: if passed { "PASSED" } else { "FAILED" }.to_string(),
                rotational: rotation.map(|rpm| rpm > 0),
                attributes: Vec::new(),
                nvme: None,
                scsi: None,
                ata: None,
            })
        })
        .collect()
}

fn kernel_errors() -> KernelErrorSummary {
    let output = match std::process::Command::new("dmesg")
        .args(["--color=never", "--level=err,crit,alert,emerg"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return KernelErrorSummary::default(),
    };
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut errors = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let category = if lower.contains("nvme") {
                "nvme"
            } else if lower.contains("ata") || lower.contains("sata") {
                "sata"
            } else if lower.contains("bcachefs")
                || lower.contains("filesystem")
                || lower.contains("i/o error")
            {
                "filesystem"
            } else if lower.contains("memory") || lower.contains("edac") || lower.contains("mce") {
                "memory"
            } else {
                "generic"
            };
            *counts.entry(category.to_string()).or_default() += 1;
            KernelError {
                timestamp_usec: 0,
                message: line.to_string(),
                category: category.to_string(),
                source: error_source(line),
            }
        })
        .collect::<Vec<_>>();
    let total_count = errors.len() as u64;
    if errors.len() > 50 {
        errors.drain(..errors.len() - 50);
    }
    let mut by_category = counts
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect::<Vec<_>>();
    by_category.sort_by(|left, right| left.category.cmp(&right.category));
    KernelErrorSummary {
        total_count,
        by_category,
        recent_errors: errors,
    }
}

fn json_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(command).is_file()))
}

fn error_source(line: &str) -> String {
    line.split_whitespace()
        .find(|word| {
            let word = word.to_ascii_lowercase();
            word.contains("nvme") || word.starts_with("ata") || word.starts_with("sd")
        })
        .map(|word| {
            word.trim_matches(|character: char| !character.is_ascii_alphanumeric())
                .to_string()
        })
        .unwrap_or_default()
}

fn read_u32(path: &str) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn history_window(range: &str) -> (i64, i64) {
    match range {
        "1h" => (3_600_000, 60_000),
        "1d" => (86_400_000, 300_000),
        "7d" => (604_800_000, 1_800_000),
        "30d" => (2_592_000_000, 7_200_000),
        _ => (300_000, 0),
    }
}

fn window_samples(
    series: &VecDeque<Sample>,
    since: i64,
    until: i64,
    bucket_ms: i64,
) -> Vec<IoSample> {
    let samples = series
        .iter()
        .filter(|sample| sample.ts >= since && sample.ts <= until);
    if bucket_ms == 0 {
        return samples
            .map(|sample| IoSample {
                ts: sample.ts,
                in_rate: sample.in_rate,
                out_rate: sample.out_rate,
            })
            .collect();
    }
    let mut buckets: BTreeMap<i64, (f64, f64, usize)> = BTreeMap::new();
    for sample in samples {
        let bucket = sample.ts / bucket_ms * bucket_ms;
        let entry = buckets.entry(bucket).or_default();
        entry.0 += sample.in_rate;
        entry.1 += sample.out_rate;
        entry.2 += 1;
    }
    buckets
        .into_iter()
        .map(|(ts, (input, output, count))| IoSample {
            ts,
            in_rate: input / count as f64,
            out_rate: output / count as f64,
        })
        .collect()
}

fn metric(out: &mut String, name: &str, value: f64) {
    let _ = writeln!(out, "# TYPE {name} gauge\n{name} {value}");
}

fn labelled_metric(out: &mut String, name: &str, label: &str, value: &str, reading: f64) {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let _ = writeln!(out, "{name}{{{label}=\"{escaped}\"}} {reading}");
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_live_system_stats_without_an_external_service() {
        let stats = system_stats();
        assert!(stats.cpu.count >= 1);
        assert!(stats.memory.used_bytes <= stats.memory.total_bytes);
    }

    #[test]
    fn history_is_recorded_and_queried_in_process() {
        let service = MetricsService::new();
        service.record("cpu", "cpu", 12.5, 0.0);
        let history = service.history("cpu", None, "5m", 0);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].name, "cpu");
        assert_eq!(history[0].samples.len(), 1);
    }
}
