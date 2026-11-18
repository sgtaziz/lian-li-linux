use super::{NvidiaMetric, SensorCategory, SensorInfo, SensorSource, Unit};
use crate::media::SensorSourceConfig;
use std::path::Path;

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

fn find_nvidia_metric(sensors: &[SensorInfo], metric: NvidiaMetric) -> Option<SensorSourceConfig> {
    let sources: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|s| {
            matches!(
                &s.source,
                SensorSource::NvidiaGpu { metric: m, .. } if *m == metric
            )
        })
        .collect();

    sources
        .iter()
        .find(|s| s.current_value.is_some())
        .or_else(|| sources.first())
        .map(|s| source_to_config(s.source.clone()))
}

/// Resolve a `SensorCategory` to a concrete `SensorSourceConfig` based on
/// what the current machine exposes. Returns `None` when no suitable sensor
/// is available.
pub fn pick_source_for_category(
    category: SensorCategory,
    sensors: &[SensorInfo],
) -> Option<SensorSourceConfig> {
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
        SensorCategory::GpuMemUsage => find_nvidia_metric(sensors, NvidiaMetric::MemUsage),
        SensorCategory::GpuMemFree => find_nvidia_metric(sensors, NvidiaMetric::MemFree),
        SensorCategory::GpuMemTotal => find_nvidia_metric(sensors, NvidiaMetric::MemTotal),
        SensorCategory::GpuClockCore => find_nvidia_metric(sensors, NvidiaMetric::ClockCore),
        SensorCategory::GpuClockMemory => find_nvidia_metric(sensors, NvidiaMetric::ClockMemory),
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

pub fn infer_sensor_category(source: &SensorSourceConfig) -> Option<SensorCategory> {
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
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::MemUsage,
            ..
        } => Some(SensorCategory::GpuMemUsage),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::MemFree,
            ..
        } => Some(SensorCategory::GpuMemFree),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::MemTotal,
            ..
        } => Some(SensorCategory::GpuMemTotal),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::ClockCore,
            ..
        } => Some(SensorCategory::GpuClockCore),
        SensorSourceConfig::NvidiaGpu {
            metric: NvidiaMetric::ClockMemory,
            ..
        } => Some(SensorCategory::GpuClockMemory),
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

fn source_to_config(source: SensorSource) -> SensorSourceConfig {
    use super::{DiskDirection, NetDirection};
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
