use super::{DiskDirection, NetDirection, NvidiaMetric, RateState, ResolvedSensor, SensorSource};
use crate::systeminfo::SysSensor;
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub fn read_sensor_value(resolved: &ResolvedSensor) -> anyhow::Result<f32> {
    match resolved {
        ResolvedSensor::SysfsFile { path, divider, .. } => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            let raw_value: f32 = content
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
            Ok(raw_value / (*divider as f32))
        }
        ResolvedSensor::Virtual { source, divider } => match source {
            SensorSource::CpuUsage => Ok(SysSensor::get_cpu_usage() as f32 / *divider as f32),
            SensorSource::MemUsage => {
                let content = std::fs::read_to_string("/proc/meminfo")
                    .map_err(|e| anyhow::anyhow!("reading /proc/meminfo: {e}"))?;
                Ok(get_mem_usage(&content))
            }
            SensorSource::MemUsed => {
                let content = std::fs::read_to_string("/proc/meminfo")
                    .map_err(|e| anyhow::anyhow!("reading /proc/meminfo: {e}"))?;
                let total = extract_mem_value(&content, "MemTotal:").unwrap_or(0.0);
                let avail = extract_mem_value(&content, "MemAvailable:").unwrap_or(0.0);
                Ok((total - avail) / *divider as f32)
            }
            SensorSource::MemFree => {
                let content = std::fs::read_to_string("/proc/meminfo")
                    .map_err(|e| anyhow::anyhow!("reading /proc/meminfo: {e}"))?;
                Ok(extract_mem_value(&content, "MemAvailable:").unwrap_or(0.0) / *divider as f32)
            }
            _ => anyhow::bail!("unexpected virtual sensor source"),
        },
        ResolvedSensor::NvidiaGpu { index, metric } => Ok(nvidia_cache_get(*index, *metric)),
        ResolvedSensor::RuntimeFile { path, max_age } => {
            if let Some(max_age) = max_age {
                let modified = std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .map_err(|e| anyhow::anyhow!("metadata {}: {e}", path.display()))?;
                let age = modified
                    .elapsed()
                    .map_err(|e| anyhow::anyhow!("timestamp {}: {e}", path.display()))?;
                if age > *max_age {
                    anyhow::bail!(
                        "runtime sensor {} is stale ({age:?} > {max_age:?})",
                        path.display()
                    );
                }
            }
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            let temp: f32 = content
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
            if !temp.is_finite() {
                anyhow::bail!("runtime sensor {} is not finite", path.display());
            }
            Ok(temp)
        }
        ResolvedSensor::ShellCommand(cmd) => {
            let output = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .output()
                .map_err(|e| anyhow::anyhow!("executing command: {e}"))?;
            if !output.status.success() {
                anyhow::bail!("command failed with status {}", output.status);
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let temp_str = stdout
                .split_whitespace()
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty output"))?;
            let temp: f32 = temp_str
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing '{temp_str}': {e}"))?;
            if !temp.is_finite() {
                anyhow::bail!("value '{temp}' is not finite");
            }
            Ok(temp)
        }
        ResolvedSensor::Constant(value) => Ok(*value),
        ResolvedSensor::NetworkRate {
            iface,
            direction,
            divider,
            state,
        } => {
            let counter = read_network_counter(iface, *direction)?;
            Ok(compute_rate(state, counter, *divider))
        }
        ResolvedSensor::DiskRate {
            device,
            direction,
            divider,
            state,
        } => {
            let counter = read_disk_counter(device, *direction)?;
            Ok(compute_rate(state, counter, *divider))
        }
    }
}

type NvidiaCache = Arc<Mutex<HashMap<(u32, NvidiaMetric), f32>>>;

static NVIDIA_CACHE: OnceLock<NvidiaCache> = OnceLock::new();

fn nvidia_cache_get(index: u32, metric: NvidiaMetric) -> f32 {
    let cache = NVIDIA_CACHE.get_or_init(|| {
        let cache: NvidiaCache = Arc::new(Mutex::new(HashMap::new()));
        let cache_clone = Arc::clone(&cache);
        std::thread::spawn(move || loop {
            if let Ok(values) = query_nvidia_smi_all() {
                *cache_clone.lock().unwrap() = values;
            }
            std::thread::sleep(Duration::from_secs(1));
        });
        cache
    });
    cache
        .lock()
        .unwrap()
        .get(&(index, metric))
        .copied()
        .unwrap_or(0.0)
}

fn query_nvidia_smi_all() -> anyhow::Result<HashMap<(u32, NvidiaMetric), f32>> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,temperature.gpu,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("nvidia-smi: {e}"))?;
    if !output.status.success() {
        anyhow::bail!("nvidia-smi failed");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let Ok(idx) = parts[0].parse::<u32>() else {
            continue;
        };
        if let Ok(temp) = parts[1].parse::<f32>() {
            map.insert((idx, NvidiaMetric::Temp), temp);
        }
        if let Ok(usage) = parts[2].parse::<f32>() {
            map.insert((idx, NvidiaMetric::Usage), usage);
        }
    }
    Ok(map)
}

fn compute_rate(state: &Arc<Mutex<RateState>>, counter: u64, divider: usize) -> f32 {
    let now = Instant::now();
    let mut s = state.lock().unwrap();
    let rate = match (s.prev_counter, s.prev_at) {
        (Some(prev), Some(prev_at)) => {
            let dt = now.saturating_duration_since(prev_at).as_secs_f32();
            if counter < prev || dt <= 0.0 {
                0.0
            } else {
                (counter - prev) as f32 / dt / divider.max(1) as f32
            }
        }
        _ => 0.0,
    };
    s.prev_counter = Some(counter);
    s.prev_at = Some(now);
    rate
}

fn read_network_counter(iface: &str, direction: NetDirection) -> anyhow::Result<u64> {
    let content = std::fs::read_to_string("/proc/net/dev")
        .map_err(|e| anyhow::anyhow!("reading /proc/net/dev: {e}"))?;
    for line in content.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() != iface {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let idx = match direction {
            NetDirection::Rx => 0,
            NetDirection::Tx => 8,
        };
        return fields
            .get(idx)
            .and_then(|f| f.parse::<u64>().ok())
            .ok_or_else(|| anyhow::anyhow!("malformed /proc/net/dev for {iface}"));
    }
    anyhow::bail!("interface '{iface}' not found in /proc/net/dev")
}

fn read_disk_counter(device: &str, direction: DiskDirection) -> anyhow::Result<u64> {
    let content = std::fs::read_to_string("/proc/diskstats")
        .map_err(|e| anyhow::anyhow!("reading /proc/diskstats: {e}"))?;
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.get(2).copied() != Some(device) {
            continue;
        }
        // sector = 512 bytes; idx 5 = sectors read, idx 9 = sectors written
        let idx = match direction {
            DiskDirection::Read => 5,
            DiskDirection::Write => 9,
        };
        let sectors: u64 = fields
            .get(idx)
            .and_then(|f| f.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("malformed /proc/diskstats for {device}"))?;
        return Ok(sectors.saturating_mul(512));
    }
    anyhow::bail!("device '{device}' not found in /proc/diskstats")
}

pub fn get_mem_usage(content: &str) -> f32 {
    let mem_total = extract_mem_value(content, "MemTotal:");
    let mem_avail = extract_mem_value(content, "MemAvailable:");
    if let (Some(total), Some(avail)) = (mem_total, mem_avail) {
        if total > 0.0 {
            return 100.0 - avail * 100.0 / total;
        }
    }
    0.0
}

fn extract_mem_value(input: &str, target: &str) -> Option<f32> {
    let line = input.lines().find(|l| l.starts_with(target))?;
    let parts: Vec<&str> = line.split_whitespace().collect();
    parts.get(1)?.parse::<f32>().ok()
}
