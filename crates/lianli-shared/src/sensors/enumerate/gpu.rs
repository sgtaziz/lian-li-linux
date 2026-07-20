//! GPU sensor enumeration: NVIDIA via `nvidia-smi`, AMD via `/sys/class/drm`.

use crate::sensors::{NvidiaMetric, SensorInfo, SensorSource, Unit};
use std::collections::HashMap;
use std::process::Command;

/// Emit `SensorInfo` records for every NVIDIA GPU reported by `nvidia-smi`.
///
/// No-op when the binary is missing or no NVIDIA GPU is present.
pub fn enumerate_nvidia(sensors: &mut Vec<SensorInfo>) {
    let Ok(output) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,temperature.gpu,utilization.gpu",
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
        if parts.len() < 4 {
            continue;
        }
        let gpu_index: u32 = parts[0].trim().parse().unwrap_or(0);
        let gpu_name = parts[1].trim();
        let temp: Option<f32> = parts[2].trim().parse().ok();
        let usage: Option<f32> = parts[3].trim().parse().ok();

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

/// Build the `pci_id → friendly name` map for AMD GPUs via `lspci`.
///
/// Strips the common prefix when multiple AMD GPUs are present (e.g. two
/// "AMD Radeon RX 7900 XT" become "RX 7900 XT").
pub fn get_amd_gpu_names() -> HashMap<String, String> {
    let mut gpus = HashMap::new();

    let output = match Command::new("lspci").output() {
        Ok(o) => o,
        Err(_) => return gpus,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

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

    clean_common_prefixes(gpus)
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
