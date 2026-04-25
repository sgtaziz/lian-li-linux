use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::systeminfo::SysSensor;

/// SensorSource stores the information of a sensor in a way so that we can store it in a file, reboot, reload the file and are still able to find the sensor.
/// In order to actually read the sensor value the implemented way is to create a ResolvedSensor from SensorSource and use that.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NvidiaMetric {
    #[default]
    Temp,
    Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetDirection {
    Rx,
    Tx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskDirection {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SensorSource {
    Hwmon {
        name: String,
        label: String,
        #[serde(default)]
        device_path: String,
    },
    NvidiaGpu {
        #[serde(default)]
        gpu_index: u32,
        #[serde(default)]
        metric: NvidiaMetric,
    },
    AmdGpuUsage {
        #[serde(default)]
        card_index: u32,
    },
    Command {
        cmd: String,
    },
    WirelessCoolant {
        device_id: String,
    },
    CpuUsage,
    MemUsage,
    MemUsed,
    MemFree,
    NetworkRate {
        iface: String,
        direction: NetDirection,
    },
    DiskRate {
        device: String,
        direction: DiskDirection,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorName {
    device_name: String,
    sensor_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Unit {
    C,
    RPM,
    V,
    FREQ,
    PERCENT,
    SIZE,
    MBps,
    WO,
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let symbol = match self {
            Unit::C => "°C",
            Unit::RPM => "RPM",
            Unit::V => "mV",
            Unit::FREQ => "Mhz",
            Unit::SIZE => "GB",
            Unit::MBps => "MB/s",
            Unit::PERCENT => "%",
            Unit::WO => "",
        };
        write!(f, "{}", symbol)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorInfo {
    pub source: SensorSource,
    pub sensor_name: Option<SensorName>,
    pub display_name: Option<String>,
    pub divider: usize,
    pub unit: Unit,
    pub current_value: Option<f32>,
}

impl SensorInfo {
    pub fn get_display_name(&self) -> String {
        self.display_name.clone().unwrap_or_else(|| {
            self.sensor_name
                .as_ref()
                .map(|s| format!("{}: {} in {}", s.device_name, s.sensor_name, self.unit))
                .unwrap_or_else(|| "Unknown Sensor".to_string())
        })
    }
}

/// ResolvedSensor is created from a SensorSource: SensorSource stores the information of a sensor in a way so that we can store it in a file, reboot, reload the file and still are able to find the sensor.
/// But in order to actually read the sensor we need to look into SensorSource thoroughly to find the real path, etc. This is how ResolvedSensor comes into play:
/// It's created from a SensorSource and enables us to read a sensor as fast as possible.

#[derive(Debug, Default)]
pub struct RateState {
    prev_counter: Option<u64>,
    prev_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub enum ResolvedSensor {
    SysfsFile {
        path: PathBuf,
        divider: usize,
    },
    NvidiaGpu {
        index: u32,
        metric: NvidiaMetric,
    },
    ShellCommand(String),
    RuntimeFile(PathBuf),
    Virtual {
        source: SensorSource,
        divider: usize,
    },
    Constant(f32),
    NetworkRate {
        iface: String,
        direction: NetDirection,
        divider: usize,
        state: Arc<Mutex<RateState>>,
    },
    DiskRate {
        device: String,
        direction: DiskDirection,
        divider: usize,
        state: Arc<Mutex<RateState>>,
    },
}

/// Abstract sensor categories used by downloadable templates to bind widgets
/// to whichever concrete sensor the user's machine exposes. Resolved once at
/// template install time into a concrete `SensorSourceConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorCategory {
    CpuTemp,
    GpuTemp,
    CpuUsage,
    GpuUsage,
    MemUsage,
    MemUsed,
    MemFree,
    NetworkRx,
    NetworkTx,
    DiskRead,
    DiskWrite,
}

/// Picks the most likely "CPU control temp" sensor — k10temp's `Tctl` /
/// coretemp's `Package id 0` first, then any other CPU temp sensor as a
/// fallback. Returns `None` if no CPU temp is exposed.
pub fn find_default_cpu_temp(sensors: &[SensorInfo]) -> Option<SensorSource> {
    let cpu_temps: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            s.unit == Unit::C
                && matches!(
                    &s.source,
                    SensorSource::Hwmon { name, .. } if name == "k10temp" || name == "coretemp"
                )
        })
        .collect();

    cpu_temps
        .iter()
        .find(|s| {
            if let SensorSource::Hwmon { label, .. } = &s.source {
                let l = label.to_lowercase();
                l.contains("tctl") || l.contains("package id 0")
            } else {
                false
            }
        })
        .or_else(|| cpu_temps.first())
        .map(|s| s.source.clone())
}

/// Picks the most likely "GPU edge temp" sensor — NVIDIA first (if present),
/// then amdgpu's `edge` label, then any GPU-ish hwmon temp.
pub fn find_default_gpu_temp(sensors: &[SensorInfo]) -> Option<SensorSource> {
    if let Some(s) = sensors.iter().find(|s| {
        matches!(
            &s.source,
            SensorSource::NvidiaGpu {
                metric: NvidiaMetric::Temp,
                ..
            }
        )
    }) {
        return Some(s.source.clone());
    }
    let gpu_temps: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            s.unit == Unit::C
                && matches!(
                    &s.source,
                    SensorSource::Hwmon { name, .. } if name == "amdgpu" || name == "radeon"
                )
        })
        .collect();
    gpu_temps
        .iter()
        .find(|s| {
            if let SensorSource::Hwmon { label, .. } = &s.source {
                label.to_lowercase().contains("edge")
            } else {
                false
            }
        })
        .or_else(|| gpu_temps.first())
        .map(|s| s.source.clone())
}

/// Resolve a `SensorCategory` to a concrete `SensorSourceConfig` based on
/// what the current machine exposes. Returns `None` when no suitable sensor
/// is available so the caller can leave the widget's existing source intact.
pub fn pick_source_for_category(
    category: SensorCategory,
    sensors: &[SensorInfo],
) -> Option<crate::media::SensorSourceConfig> {
    use crate::media::SensorSourceConfig;
    match category {
        SensorCategory::CpuUsage => Some(SensorSourceConfig::CpuUsage),
        SensorCategory::MemUsage => Some(SensorSourceConfig::MemUsage),
        SensorCategory::MemUsed => Some(SensorSourceConfig::MemUsed),
        SensorCategory::MemFree => Some(SensorSourceConfig::MemFree),
        SensorCategory::CpuTemp => find_default_cpu_temp(sensors).map(source_to_config),
        SensorCategory::GpuTemp => find_default_gpu_temp(sensors).map(source_to_config),
        SensorCategory::GpuUsage => sensors
            .iter()
            .find(|s| {
                matches!(
                    &s.source,
                    SensorSource::NvidiaGpu {
                        metric: NvidiaMetric::Usage,
                        ..
                    } | SensorSource::AmdGpuUsage { .. }
                )
            })
            .map(|s| source_to_config(s.source.clone())),
        SensorCategory::NetworkRx => {
            default_route_iface().map(|iface| SensorSourceConfig::NetworkRx { iface })
        }
        SensorCategory::NetworkTx => {
            default_route_iface().map(|iface| SensorSourceConfig::NetworkTx { iface })
        }
        SensorCategory::DiskRead => {
            root_disk_device().map(|device| SensorSourceConfig::DiskRead { device })
        }
        SensorCategory::DiskWrite => {
            root_disk_device().map(|device| SensorSourceConfig::DiskWrite { device })
        }
    }
}

pub fn infer_sensor_category(source: &crate::media::SensorSourceConfig) -> Option<SensorCategory> {
    use crate::media::SensorSourceConfig;
    match source {
        SensorSourceConfig::CpuUsage => Some(SensorCategory::CpuUsage),
        SensorSourceConfig::MemUsage => Some(SensorCategory::MemUsage),
        SensorSourceConfig::MemUsed => Some(SensorCategory::MemUsed),
        SensorSourceConfig::MemFree => Some(SensorCategory::MemFree),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::Temp,
            ..
        } => Some(SensorCategory::GpuTemp),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::Usage,
            ..
        } => Some(SensorCategory::GpuUsage),
        SensorSourceConfig::AmdGpuUsage { .. } => Some(SensorCategory::GpuUsage),
        SensorSourceConfig::Hwmon { name, label, .. } => {
            let l = label.to_lowercase();
            if name == "k10temp" || name == "coretemp" {
                if l.contains("tctl") || l.contains("package id 0") || l.starts_with("core") {
                    return Some(SensorCategory::CpuTemp);
                }
                return Some(SensorCategory::CpuTemp);
            }
            if (name == "amdgpu" || name == "radeon") && (l.contains("edge") || l.contains("temp"))
            {
                return Some(SensorCategory::GpuTemp);
            }
            None
        }
        SensorSourceConfig::NetworkRx { .. } => Some(SensorCategory::NetworkRx),
        SensorSourceConfig::NetworkTx { .. } => Some(SensorCategory::NetworkTx),
        SensorSourceConfig::DiskRead { .. } => Some(SensorCategory::DiskRead),
        SensorSourceConfig::DiskWrite { .. } => Some(SensorCategory::DiskWrite),
        SensorSourceConfig::Command { .. }
        | SensorSourceConfig::Constant { .. }
        | SensorSourceConfig::WirelessCoolant { .. } => None,
    }
}

fn source_to_config(source: SensorSource) -> crate::media::SensorSourceConfig {
    use crate::media::SensorSourceConfig;
    match source {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name,
            label,
            device_path,
        },
        SensorSource::NvidiaGpu { gpu_index, metric } => {
            SensorSourceConfig::NvidiaGpu { gpu_index, metric }
        }
        SensorSource::AmdGpuUsage { card_index } => SensorSourceConfig::AmdGpuUsage { card_index },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd },
        SensorSource::WirelessCoolant { device_id } => {
            SensorSourceConfig::WirelessCoolant { device_id }
        }
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
        SensorSource::NetworkRate { iface, direction } => match direction {
            NetDirection::Rx => SensorSourceConfig::NetworkRx { iface },
            NetDirection::Tx => SensorSourceConfig::NetworkTx { iface },
        },
        SensorSource::DiskRate { device, direction } => match direction {
            DiskDirection::Read => SensorSourceConfig::DiskRead { device },
            DiskDirection::Write => SensorSourceConfig::DiskWrite { device },
        },
    }
}

pub fn enumerate_sensors() -> Vec<SensorInfo> {
    let mut sensors = Vec::new();

    let mut mem_idx: usize = 0;
    let mut gfx_idx: usize = 0;
    let gpu_names = get_amd_gpu_names();

    sensors.push(SensorInfo {
        source: SensorSource::CpuUsage,
        sensor_name: None,
        display_name: Some("CPU: Usage".to_string()),
        divider: 100,
        unit: Unit::PERCENT,
        current_value: Some(0.0),
    });
    sensors.push(SensorInfo {
        source: SensorSource::MemUsage,
        sensor_name: None,
        display_name: Some("RAM: Usage".to_string()),
        divider: 1,
        unit: Unit::PERCENT,
        current_value: Some(0.0),
    });
    sensors.push(SensorInfo {
        source: SensorSource::MemUsed,
        sensor_name: None,
        display_name: Some("RAM: Used".to_string()),
        divider: 1024 * 1024,
        unit: Unit::SIZE,
        current_value: Some(0.0),
    });
    sensors.push(SensorInfo {
        source: SensorSource::MemFree,
        sensor_name: None,
        display_name: Some("RAM: Free".to_string()),
        divider: 1024 * 1024,
        unit: Unit::SIZE,
        current_value: Some(0.0),
    });

    // Scan hwmon devices
    let hwmon_path = "/sys/class/hwmon/";
    if let Ok(entries) = std::fs::read_dir(hwmon_path) {
        let mut sorted_entries: Vec<_> = entries.flatten().collect();
        // Sort, so that hwmon<x> is numerically correct sorted (especially hwmon10 after hwmon9)

        sorted_entries.sort_by_cached_key(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .strip_prefix("hwmon")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        for entry in sorted_entries {
            let path = entry.path();
            let name = match std::fs::read_to_string(path.join("name")) {
                Ok(n) => n.trim().to_string(),
                Err(_) => continue,
            };

            let pci_id = get_pci_id_from_path(path.clone());
            let pci_id_stripped = pci_id.strip_prefix("0000:").unwrap_or(&pci_id).to_string();

            let result = get_display_name(&path, &pci_id_stripped, &gpu_names, mem_idx, gfx_idx);
            mem_idx = result.1;
            gfx_idx = result.2;
            let display_name = match result.0 {
                Some(name) => name,
                None => continue,
            };

            let device_path = std::fs::read_link(path.join("device"))
                .ok()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));

            if let Ok(files) = std::fs::read_dir(&path) {
                let mut device_sensors: Vec<SensorInfo> = Vec::new();

                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().to_string();
                    if fname.ends_with("_input") {
                        // Extract prefix (filename might be freq1_input). Prefix contains useful hints for what kind of value it is (Frequency, Voltage, Power, °C etc)
                        let prefix = fname.strip_suffix("_input").unwrap();
                        let label = std::fs::read_to_string(path.join(format!("{}_label", prefix)))
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|_| "".to_string());
                        let display_label = get_label_name(prefix, &label);
                        let (unit, divider) = get_unit(prefix);
                        let value = read_sysfs_file(&file.path()).map(|v| v / divider as f32);
                        let sensor_name = Some(SensorName {
                            device_name: display_name.clone(),
                            sensor_name: display_label,
                        });
                        let device_path = if let Some(dev) = &device_path {
                            if dev.starts_with("DEADBEEF") {
                                // virtual devices (for example my motherboard from Gigabyte links to "DEADBEEF-2001-0000-00A0-C90629100000")
                                pci_id.to_string()
                            } else {
                                dev.to_string()
                            }
                        } else {
                            pci_id.to_string()
                        };

                        device_sensors.push(SensorInfo {
                            source: SensorSource::Hwmon {
                                name: name.clone(),
                                label: prefix.to_string(),
                                device_path,
                            },
                            sensor_name,
                            display_name: None,
                            divider,
                            unit,
                            current_value: value,
                        });
                    }
                }

                device_sensors.sort_by_cached_key(|s| s.get_display_name());
                sensors.extend(device_sensors);
            }
        }
    }

    // Check for NVIDIA GPU
    if let Ok(output) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,temperature.gpu,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(", ").collect();
                if parts.len() >= 4 {
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
        }
    }

    enumerate_amd_gpu_usage(&gpu_names, &mut sensors);
    enumerate_network_sensors(&mut sensors);
    enumerate_disk_sensors(&mut sensors);

    sensors
}

fn enumerate_amd_gpu_usage(gpu_names: &HashMap<String, String>, sensors: &mut Vec<SensorInfo>) {
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

pub fn get_pci_id_from_path(hwmon_path: PathBuf) -> String {
    // 1. Resolve that 'device'-link in the hwmon-folder
    // canonicalize() creates a true absolute path from a symlink
    let device_path = hwmon_path.join("device");

    let opt_full_path = std::fs::canonicalize(device_path).ok();
    if opt_full_path.is_none() {
        return "None".to_string();
    }

    let full_path = opt_full_path.unwrap();

    // 2. Iterate over the components of path from right to left
    // A path might look as follows: /sys/devices/pci0000:00/0000:00:01.1/0000:04:00.0/nvme/nvme1
    for component in full_path.ancestors() {
        if let Some(name_os) = component.file_name() {
            let name = name_os.to_string_lossy();

            // Validation: A PCI-ID has the format Domain:Bus:Device.Function
            // Let's check for these typical separators and minimum length (e.g. 00:00.0)
            if name.contains(':') && name.contains('.') && name.len() >= 7 {
                return name.into_owned();
            }
        }

        // stop if we are about to leave the kernel device tree
        if component == Path::new("/sys/devices") {
            break;
        }
    }

    let name = std::fs::read_to_string(hwmon_path.join("name"))
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();

    // We'll return "platform:NAME", in order to make it distinguishable from our PCI-IDs
    format!("platform:{}", name)
}

/// retrieves human readable description of a metric and returns default values for this metric
/// Returns desc, unit, divider

fn get_unit(prefix: &str) -> (Unit, usize) {
    if prefix.starts_with("temp") {
        (Unit::C, 1000)
    } else if prefix.starts_with("fan") {
        (Unit::RPM, 1)
    } else if prefix.starts_with("in") {
        (Unit::V, 1)
    } else if prefix.starts_with("freq") {
        (Unit::FREQ, 1000 * 1000)
    } else {
        (Unit::WO, 1)
    }
}

/// Creates a meaningful name (human readable) for a sensor
/// prefix is the name of the sensor file (without _input), and label is the content of the <prefix>_label file
/// Note that label can be empty!

pub fn get_label_name(prefix: &str, label: &str) -> String {
    let lower_label = label.to_lowercase();
    let lower_prefix = prefix.to_lowercase();
    // Dynamic replacements for CCDs and Cores
    let new_label = if lower_label.ends_with("ctl") {
        "Control Temp".to_string()
    } else if lower_label.ends_with("package id 0") {
        "Control Temp".to_string()
    } else if lower_label.ends_with("junction") && lower_prefix.starts_with("temp") {
        "Hotspot Temp".to_string()
    } else if lower_label.ends_with("edge") && lower_prefix.starts_with("temp") {
        "Edge Temp".to_string()
    } else if lower_label.ends_with("mem") && lower_prefix.starts_with("temp") {
        "VRAM Temp".to_string()
    } else if lower_label.ends_with("sclk") && lower_prefix.starts_with("freq") {
        "System Clock".to_string()
    } else if lower_label.ends_with("mclk") && lower_prefix.starts_with("freq") {
        "Memory Clock".to_string()
    } else if lower_label.ends_with("vddgfx") && lower_prefix.starts_with("in") {
        "GPU Voltage".to_string()
    } else if let Some(idx) = lower_label.find("ccd") {
        format!("Temp Die {}", &lower_label[idx + 3..])
    } else if let Some(idx) = lower_label.find("core ") {
        format!("Temp Core {}", &lower_label[idx + 5..])
    } else if let Some(idx) = lower_label.find("fan") {
        format!("Fan {}", &lower_label[idx + 3..])
    } else if let Some(idx) = lower_prefix.find("fan") {
        format!("Fan {}", &lower_prefix[idx + 3..])
    } else if label.is_empty() {
        prefix.to_string()
    } else {
        label.to_string()
    };

    new_label
}

fn get_amd_gpu_names() -> HashMap<String, String> {
    let mut gpus = HashMap::new();

    let output = match Command::new("lspci").output() {
        Ok(o) => o,
        Err(_) => return gpus,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let line_lower = line.to_lowercase();

        // Now let's filter: Must contain "vga" (or "display"/"3d") and "amd"
        if (line_lower.contains("vga") || line_lower.contains("display"))
            && line_lower.contains("amd")
        {
            // The PCI-address is always at the beginning (first 7-12 chars)
            // Example : "03:00.0 VGA compatible controller: ..."
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

// Helper method: Looks at all the values in the hash map. If all of them share a single prefix, then remove the prefix
// For example, if all values contain the prefix "VGA compatible controller: <blah blah>", then the prefix "VGA compatible controller: " will be removed

fn clean_common_prefixes(mut gpus: HashMap<String, String>) -> HashMap<String, String> {
    if gpus.len() <= 1 {
        return gpus;
    }

    // 1. step: Find the prefix common to all values:

    let values: Vec<&String> = gpus.values().collect();
    // We'll use the first name as base for our comparison
    let mut common_prefix = values[0].clone();

    for name in values.iter().skip(1) {
        // Now we shorten the common prefix until it's also a common prefix from 'name'
        while !name.starts_with(&common_prefix) && !common_prefix.is_empty() {
            common_prefix.pop();
        }
    }
    // Ok, common prefix found!

    // 2. step: Remove the common prefix from all values
    if !common_prefix.is_empty() {
        let prefix_len = common_prefix.len();
        for value in gpus.values_mut() {
            *value = value[prefix_len..].trim().to_string();
        }
    }

    gpus
}

// Creates a human readable name from this path
// Needs the current pci id in order to find the name of the graphics card
// Needs the number of found graphics cards in order to process only the first one (main graphics card, external one) and ignore the second one (secondary graphics card, CPU with built-in GPU)
// Needs the number of RAM modules found so far to enumerate the RAM modules.

pub fn get_display_name(
    hwmon_path: &Path,
    pci_id_stripped: &str,
    gpu_names: &HashMap<String, String>,
    mem_idx: usize,
    gfx_idx: usize,
) -> (Option<String>, usize, usize) {
    // First of all, check for file device/model and return that in case it exists and contains something
    // This will display for example the type and name of the nvme-SSD
    let model_path = hwmon_path.join("device").join("model");

    if let Ok(model_name) = std::fs::read_to_string(model_path) {
        return (Some(model_name.trim().to_string()), mem_idx, gfx_idx);
    }

    if let Ok(generic_name) = std::fs::read_to_string(hwmon_path.join("name")) {
        let name = generic_name.trim();
        if name == "nvme" {
            return (Some("NVMe Storage Device".to_string()), mem_idx, gfx_idx);
        }
        if name == "k10temp" || name == "coretemp" {
            return (Some("CPU".to_string()), mem_idx, gfx_idx);
        }
        if name == "amdgpu" {
            if let Some(gpu_name) = gpu_names.get(pci_id_stripped) {
                return (Some(gpu_name.clone()), mem_idx, gfx_idx + 1);
            }
            return (Some(format!("AMD GPU {}", gfx_idx)), mem_idx, gfx_idx + 1);
        }
        if name == "nouveau" {
            return (Some("NVidia GPU".to_string()), mem_idx, gfx_idx + 1);
        }
        let common_drivers = ["nct", "it8", "f71", "gigabyte_wmi", "w83"];
        if common_drivers.iter().any(|&d| name.starts_with(d)) {
            return (Some("Motherboard".to_string()), mem_idx, gfx_idx);
        }
        if name.starts_with("spd") {
            return (
                Some(format!("DDR5 RAM Module {}", mem_idx + 1)),
                mem_idx + 1,
                gfx_idx,
            );
        }
        if name.starts_with("ee1004") {
            return (
                Some(format!("DDR4 RAM Module {}", mem_idx + 1)),
                mem_idx + 1,
                gfx_idx,
            );
        }
        if name.starts_with("jc42") {
            return (
                Some(format!("DDR3/ECC RAM Module {}", mem_idx + 1)),
                mem_idx + 1,
                gfx_idx,
            );
        }
        if name == "acpitz" {
            return (None, mem_idx, gfx_idx);
        }

        (Some(name.to_string()), mem_idx, gfx_idx)
    } else {
        (Some("Unknown Device".to_string()), mem_idx, gfx_idx)
    }
}

pub fn resolve_sensor(source: &SensorSource, divider: usize) -> Option<ResolvedSensor> {
    match source {
        SensorSource::CpuUsage
        | SensorSource::MemUsage
        | SensorSource::MemUsed
        | SensorSource::MemFree => Some(ResolvedSensor::Virtual {
            source: source.clone(),
            divider,
        }),
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => {
            let hwmon_dir = Path::new("/sys/class/hwmon");
            let entries = std::fs::read_dir(hwmon_dir).ok()?;

            for entry in entries.flatten() {
                let path = entry.path();

                // Match by device_path (PCI ID) when available, otherwise by hwmon name
                if device_path.is_empty() {
                    let hw_name = std::fs::read_to_string(path.join("name"))
                        .ok()
                        .map(|n| n.trim().to_string());
                    if hw_name.as_deref() != Some(name) {
                        continue;
                    }
                } else {
                    let device_path_symlink = std::fs::read_link(path.join("device"))
                        .ok()
                        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));

                    let curr_device_path = if let Some(dev) = &device_path_symlink {
                        if dev.starts_with("DEADBEEF") {
                            get_pci_id_from_path(path.clone())
                        } else {
                            dev.to_string()
                        }
                    } else {
                        get_pci_id_from_path(path.clone())
                    };

                    if curr_device_path != *device_path {
                        continue;
                    }
                }

                if let Ok(files) = std::fs::read_dir(&path) {
                    for file in files.flatten() {
                        let fname = file.file_name().to_string_lossy().to_string();
                        if fname.ends_with("_input") {
                            let prefix = fname.strip_suffix("_input").unwrap();
                            if prefix == label {
                                return Some(ResolvedSensor::SysfsFile {
                                    path: file.path(),
                                    divider,
                                });
                            }
                            // Old config format: label is human-readable (e.g. "Package id 0")
                            let file_label =
                                std::fs::read_to_string(path.join(format!("{prefix}_label")))
                                    .map(|l| l.trim().to_string())
                                    .unwrap_or_default();
                            if file_label == *label {
                                let actual_divider = get_unit(prefix).1;
                                return Some(ResolvedSensor::SysfsFile {
                                    path: file.path(),
                                    divider: actual_divider,
                                });
                            }
                        }
                    }
                }
            }
            None
        }
        SensorSource::NvidiaGpu { gpu_index, metric } => Some(ResolvedSensor::NvidiaGpu {
            index: *gpu_index,
            metric: *metric,
        }),
        SensorSource::AmdGpuUsage { card_index } => {
            let path = PathBuf::from(format!(
                "/sys/class/drm/card{card_index}/device/gpu_busy_percent"
            ));
            if path.exists() {
                Some(ResolvedSensor::SysfsFile { path, divider: 1 })
            } else {
                None
            }
        }
        SensorSource::Command { cmd } => Some(ResolvedSensor::ShellCommand(cmd.clone())),
        SensorSource::WirelessCoolant { device_id } => {
            let path = coolant_runtime_path(device_id);
            if path.exists() {
                Some(ResolvedSensor::RuntimeFile(path))
            } else {
                None
            }
        }
        SensorSource::NetworkRate { iface, direction } => Some(ResolvedSensor::NetworkRate {
            iface: iface.clone(),
            direction: *direction,
            divider,
            state: Arc::new(Mutex::new(RateState::default())),
        }),
        SensorSource::DiskRate { device, direction } => Some(ResolvedSensor::DiskRate {
            device: device.clone(),
            direction: *direction,
            divider,
            state: Arc::new(Mutex::new(RateState::default())),
        }),
    }
}

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
        ResolvedSensor::RuntimeFile(path) => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            let temp: f32 = content
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
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
        // 0=rx_bytes, 8=tx_bytes
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
        // 5=sectors read, 9=sectors written; sector = 512 bytes
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

fn default_route_iface() -> Option<String> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[1] == "00000000" {
            return Some(fields[0].to_string());
        }
    }
    None
}

fn root_disk_device() -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let mut root_dev: Option<String> = None;
    for line in mounts.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[1] == "/" {
            root_dev = Some(fields[0].to_string());
            break;
        }
    }
    let dev = root_dev?;
    let partition = dev.strip_prefix("/dev/")?.to_string();
    let block = Path::new("/sys/class/block").join(&partition);
    let canon = std::fs::canonicalize(&block).ok()?;
    let parent = canon.parent()?;
    let parent_name = parent.file_name()?.to_string_lossy().to_string();
    if parent_name == "block" {
        Some(partition)
    } else {
        Some(parent_name)
    }
}

fn enumerate_network_sensors(sensors: &mut Vec<SensorInfo>) {
    let Ok(content) = std::fs::read_to_string("/proc/net/dev") else {
        return;
    };
    let mut ifaces: Vec<String> = Vec::new();
    for line in content.lines() {
        let Some((name, _)) = line.split_once(':') else {
            continue;
        };
        let trimmed = name.trim();
        if trimmed == "lo" || trimmed.is_empty() {
            continue;
        }
        ifaces.push(trimmed.to_string());
    }
    ifaces.sort();
    for iface in ifaces {
        sensors.push(SensorInfo {
            source: SensorSource::NetworkRate {
                iface: iface.clone(),
                direction: NetDirection::Rx,
            },
            sensor_name: None,
            display_name: Some(format!("Network {iface}: Rx")),
            divider: 1_000_000,
            unit: Unit::MBps,
            current_value: Some(0.0),
        });
        sensors.push(SensorInfo {
            source: SensorSource::NetworkRate {
                iface: iface.clone(),
                direction: NetDirection::Tx,
            },
            sensor_name: None,
            display_name: Some(format!("Network {iface}: Tx")),
            divider: 1_000_000,
            unit: Unit::MBps,
            current_value: Some(0.0),
        });
    }
}

fn enumerate_disk_sensors(sensors: &mut Vec<SensorInfo>) {
    let Ok(entries) = std::fs::read_dir("/sys/block") else {
        return;
    };
    let mut devices: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let skip = name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("dm-")
                || name.starts_with("zram")
                || name.starts_with("sr");
            if skip {
                None
            } else {
                Some(name)
            }
        })
        .collect();
    devices.sort();
    for device in devices {
        sensors.push(SensorInfo {
            source: SensorSource::DiskRate {
                device: device.clone(),
                direction: DiskDirection::Read,
            },
            sensor_name: None,
            display_name: Some(format!("Disk {device}: Read")),
            divider: 1_000_000,
            unit: Unit::MBps,
            current_value: Some(0.0),
        });
        sensors.push(SensorInfo {
            source: SensorSource::DiskRate {
                device: device.clone(),
                direction: DiskDirection::Write,
            },
            sensor_name: None,
            display_name: Some(format!("Disk {device}: Write")),
            divider: 1_000_000,
            unit: Unit::MBps,
            current_value: Some(0.0),
        });
    }
}

/// Runtime path for a wireless coolant temperature file.
pub fn coolant_runtime_path(device_id: &str) -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let sanitized = device_id.replace(':', "-");
    PathBuf::from(format!("{runtime_dir}/lianli-coolant-{sanitized}"))
}

/// Write a coolant temperature value to the runtime file.
pub fn write_coolant_temp(device_id: &str, temp_c: f32) {
    let path = coolant_runtime_path(device_id);
    let _ = std::fs::write(&path, format!("{temp_c}"));
}

fn read_sysfs_file(path: &Path) -> Option<f32> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: f32 = content.trim().parse().ok()?;
    Some(value)
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

#[derive(Debug, Clone)]
pub struct PwmHeader {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
}

pub fn enumerate_pwm_headers() -> Vec<PwmHeader> {
    let gpu_names = get_amd_gpu_names();
    let mut headers = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return headers;
    };
    let mut mem_idx = 0usize;
    let mut gfx_idx = 0usize;
    for entry in entries.flatten() {
        let dir = entry.path();
        let pci_id = dir
            .join("device")
            .read_link()
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_default()
            .replace("0000:", "");
        let (friendly, mi, gi) = get_display_name(&dir, &pci_id, &gpu_names, mem_idx, gfx_idx);
        mem_idx = mi;
        gfx_idx = gi;
        let chip_label = friendly.unwrap_or_else(|| {
            std::fs::read_to_string(dir.join("name"))
                .unwrap_or_default()
                .trim()
                .to_string()
        });
        for i in 1..=10 {
            let pwm_path = dir.join(format!("pwm{i}"));
            if !pwm_path.exists() {
                break;
            }
            let hwmon = dir.file_name().unwrap_or_default().to_string_lossy();
            let id = format!("{hwmon}/pwm{i}");
            let label = format!("{chip_label} Fan{i}");
            headers.push(PwmHeader {
                id,
                label,
                path: pwm_path,
            });
        }
    }
    headers.sort_by(|a, b| a.id.cmp(&b.id));
    headers
}

pub fn read_pwm_header(id: &str) -> Option<u8> {
    let path = Path::new("/sys/class/hwmon").join(id);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
}
