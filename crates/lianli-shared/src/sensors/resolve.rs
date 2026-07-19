use super::enumerate::{pci_id_from_path, unit_for};
use super::{RateState, ResolvedSensor, SensorSource};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
                            pci_id_from_path(&path)
                        } else {
                            dev.to_string()
                        }
                    } else {
                        pci_id_from_path(&path)
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
                                let actual_divider = unit_for(prefix).1;
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
