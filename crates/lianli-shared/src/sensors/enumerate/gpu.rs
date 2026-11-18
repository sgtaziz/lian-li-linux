//! GPU sensor enumeration: NVIDIA via `nvidia-smi`, AMD via `/sys/class/drm`.

use crate::sensors::{NvidiaMetric, SensorInfo, SensorSource, Unit};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Emit `SensorInfo` records for every NVIDIA GPU reported by `nvidia-smi`.
///
/// No-op when the binary is missing or no NVIDIA GPU is present.
pub fn enumerate_nvidia(sensors: &mut Vec<SensorInfo>) {
    let Ok(output) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,temperature.gpu,utilization.gpu,memory.used,memory.free,memory.total,clocks.current.graphics,clocks.current.memory",
            "--format=csv,noheader,nounits",
        ])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(", ").collect();
        if parts.len() < 9 {
            continue;
        }
        let gpu_index: u32 = parts[0].trim().parse().unwrap_or(0);
        let gpu_name = parts[1].trim();
        let temp: Option<f32> = parts[2].trim().parse().ok();
        let usage: Option<f32> = parts[3].trim().parse().ok();
        let mem_used: Option<f32> = parts[4].trim().parse().ok();
        let mem_free: Option<f32> = parts[5].trim().parse().ok();
        let mem_total: Option<f32> = parts[6].trim().parse().ok();
        let clock_core: Option<f32> = parts[7].trim().parse().ok();
        let clock_memory: Option<f32> = parts[8].trim().parse().ok();

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::Temp,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Temp")),
            current_value: temp,
            unit: Unit::C,
            divider: 1,
        });

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::Usage,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Usage")),
            current_value: usage,
            unit: Unit::PERCENT,
            divider: 1,
        });

        let mem_usage = match (mem_used, mem_total) {
            (Some(used), Some(total)) if total > 0.0 => Some(used / total * 100.0),
            _ => None,
        };
        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::MemUsage,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Mem Usage")),
            current_value: mem_usage,
            unit: Unit::PERCENT,
            divider: 1,
        });

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::MemFree,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Mem Free")),
            current_value: mem_free,
            unit: Unit::MiB,
            divider: 1,
        });

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::MemTotal,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Mem Total")),
            current_value: mem_total,
            unit: Unit::MiB,
            divider: 1,
        });

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::ClockCore,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Core Clock")),
            current_value: clock_core,
            unit: Unit::FREQ,
            divider: 1,
        });

        sensors.push(SensorInfo {
            source: SensorSource::NvidiaGpu {
                gpu_index,
                metric: NvidiaMetric::ClockMemory,
            },
            sensor_name: None,
            display_name: Some(format!("{gpu_name}: Memory Clock")),
            current_value: clock_memory,
            unit: Unit::FREQ,
            divider: 1,
        });
    }
}

/// Emit `SensorInfo` records for every AMD GPU exposing `gpu_busy_percent`.
///
/// Walks `/sys/class/drm/cardN/device/gpu_busy_percent` and filters by vendor
/// `0x1002` (AMD). Names come from the pre-computed `gpu_names` map (keyed by
/// PCI ID with the `0000:` prefix stripped).
pub fn enumerate_amd(gpu_names: &HashMap<String, String>, sensors: &mut Vec<SensorInfo>) {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return;
    };
    let mut cards: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let idx: u32 = name.strip_prefix("card")?.parse().ok()?;
            Some((idx, e.path()))
        })
        .collect();
    cards.sort_by_key(|(idx, _)| *idx);

    for (card_index, card_path) in cards {
        let busy_path = card_path.join("device/gpu_busy_percent");
        if !busy_path.exists() {
            continue;
        }
        let vendor = std::fs::read_to_string(card_path.join("device/vendor"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if vendor != "0x1002" {
            continue;
        }

        let pci_id = std::fs::read_link(card_path.join("device"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
            .and_then(|s| s.strip_prefix("0000:").map(|t| t.to_string()));
        let name = pci_id
            .as_ref()
            .and_then(|id| gpu_names.get(id).cloned())
            .unwrap_or_else(|| format!("AMD GPU {card_index}"));

        let current_value = std::fs::read_to_string(&busy_path)
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());

        sensors.push(SensorInfo {
            source: SensorSource::AmdGpuUsage { card_index },
            sensor_name: None,
            display_name: Some(format!("{name}: Usage")),
            current_value,
            unit: Unit::PERCENT,
            divider: 1,
        });
    }
}

#[derive(Default)]
struct GpuNames {
    names: HashMap<String, String>,
    topology: Option<Vec<std::ffi::OsString>>,
    checked: Option<Instant>,
    running: bool,
}

impl GpuNames {
    fn needs_refresh(&self, topology: &[std::ffi::OsString]) -> bool {
        !self.running && self.topology.as_deref() != Some(topology)
    }

    fn finish(
        &mut self,
        topology: Vec<std::ffi::OsString>,
        names: anyhow::Result<HashMap<String, String>>,
    ) {
        if let Ok(names) = names {
            self.names = names;
            self.topology = Some(topology);
        }
        self.running = false;
    }
}

static GPU_NAMES: OnceLock<Mutex<GpuNames>> = OnceLock::new();

/// Returns cached labels while changed PCI topology is refreshed in the background.
pub fn get_amd_gpu_names() -> HashMap<String, String> {
    let cache = GPU_NAMES.get_or_init(|| Mutex::new(GpuNames::default()));
    let mut state = cache.lock().unwrap_or_else(|error| error.into_inner());
    if state.running
        || state
            .checked
            .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
    {
        return state.names.clone();
    }
    state.checked = Some(Instant::now());
    let names = state.names.clone();
    drop(state);
    let topology = std::fs::read_dir("/sys/bus/pci/devices").and_then(|entries| {
        let mut topology = entries
            .take(4096)
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        topology.sort();
        Ok(topology)
    });
    let Ok(topology) = topology else { return names };
    let mut state = cache.lock().unwrap_or_else(|error| error.into_inner());
    if !state.needs_refresh(&topology) {
        return names;
    }
    state.running = true;
    drop(state);
    // A kernel-stuck PCI query must not block control or shutdown. Keep at most one worker until it can be reaped.
    if let Err(error) = std::thread::Builder::new()
        .name("pci-labels".into())
        .spawn(move || {
            let result = load_amd_gpu_names();
            cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .finish(topology, result);
        })
    {
        cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .running = false;
        tracing::warn!("Cannot refresh GPU labels: {error}");
    }
    names
}

fn load_amd_gpu_names() -> anyhow::Result<HashMap<String, String>> {
    let mut command = Command::new("lspci");
    command.args(["-d", "1002:"]).env("LC_ALL", "C");
    let output = crate::sensors::command::output(command)?;
    let stdout = String::from_utf8_lossy(&output);
    let mut gpus = HashMap::new();

    for line in stdout.lines() {
        let line_lower = line.to_lowercase();
        if (line_lower.contains("vga") || line_lower.contains("display"))
            && line_lower.contains("amd")
        {
            if let Some((addr, full_desc)) = line.split_once(' ') {
                let clean_name = if let Some((_, actual_name)) = full_desc.split_once(": ") {
                    actual_name.trim()
                } else {
                    full_desc.trim()
                };
                gpus.insert(addr.to_string(), clean_name.to_string());
            }
        }
    }

    Ok(clean_common_prefixes(gpus))
}

fn clean_common_prefixes(mut gpus: HashMap<String, String>) -> HashMap<String, String> {
    if gpus.len() <= 1 {
        return gpus;
    }

    let values: Vec<&String> = gpus.values().collect();
    let mut common_prefix = values[0].clone();

    for name in values.iter().skip(1) {
        while !name.starts_with(&common_prefix) && !common_prefix.is_empty() {
            common_prefix.pop();
        }
    }

    if !common_prefix.is_empty() {
        let prefix_len = common_prefix.len();
        for value in gpus.values_mut() {
            *value = value[prefix_len..].trim().to_string();
        }
    }

    gpus
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn topology_changes_refresh_labels_without_losing_successful_results_on_failure() {
        let first = vec!["0000:01:00.0".into()];
        let second = vec!["0000:02:00.0".into()];
        let mut cache = GpuNames::default();
        assert!(cache.needs_refresh(&first));
        cache.running = true;
        assert!(!cache.needs_refresh(&second));
        let names = HashMap::from([("01:00.0".into(), "AMD GPU".into())]);
        cache.finish(first.clone(), Ok(names.clone()));
        assert_eq!(cache.names, names);
        assert!(!cache.needs_refresh(&first));
        assert!(cache.needs_refresh(&second));
        cache.running = true;
        cache.finish(second.clone(), Err(anyhow::anyhow!("query timed out")));
        assert_eq!(cache.names, names);
        assert_eq!(cache.topology, Some(first));
        assert!(cache.needs_refresh(&second));
        cache.finish(second.clone(), Ok(HashMap::new()));
        assert!(cache.names.is_empty());
        assert!(!cache.needs_refresh(&second));
    }
}
