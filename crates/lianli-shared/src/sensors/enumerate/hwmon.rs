//! `/sys/class/hwmon` walking and per-sensor metadata.
//!
//! This module knows how to:
//! - iterate the sorted list of hwmon devices,
//! - classify each one into a friendly display name ("CPU", "AMD GPU 0", …),
//! - read the per-sensor `_input` files and produce `SensorInfo` records,
//! - expose PWM fan-header enumeration on top of the same hwmon walk.
//!
//! The top-level `enumerate::enumerate_sensors()` orchestrator calls
//! [`walk_hwmon`] for the hwmon portion of the bus.

use crate::sensors::{PwmHeader, SensorInfo, SensorName, SensorSource, Unit};
use std::collections::HashMap;
use std::path::Path;

/// Walk `/sys/class/hwmon` and emit one `SensorInfo` per `*_input` file found.
///
/// `gpu_names` is the pre-computed `pci_id → friendly name` map produced by
/// [`super::gpu::get_amd_gpu_names`]; pass an empty map if GPU lookup isn't
/// needed.
pub fn walk_hwmon(gpu_names: &HashMap<String, String>) -> Vec<SensorInfo> {
    let mut out = Vec::new();
    let mut mem_idx: usize = 0;
    let mut gfx_idx: usize = 0;

    let hwmon_path = "/sys/class/hwmon/";
    let Ok(entries) = std::fs::read_dir(hwmon_path) else {
        return out;
    };
    let mut sorted_entries: Vec<_> = entries.flatten().collect();
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

        let pci_id = pci_id_from_path(&path);
        let pci_id_stripped = pci_id.strip_prefix("0000:").unwrap_or(&pci_id).to_string();

        let result = display_name(&path, &pci_id_stripped, gpu_names, mem_idx, gfx_idx);
        mem_idx = result.1;
        gfx_idx = result.2;
        let device_display_name = match result.0 {
            Some(n) => n,
            None => continue,
        };

        let device_path = std::fs::read_link(path.join("device"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));

        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };

        let mut device_sensors: Vec<SensorInfo> = Vec::new();
        for file in files.flatten() {
            let fname = file.file_name().to_string_lossy().to_string();
            if !fname.ends_with("_input") {
                continue;
            }
            let prefix = fname.strip_suffix("_input").unwrap();
            let label = std::fs::read_to_string(path.join(format!("{}_label", prefix)))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "".to_string());
            let display_label = label_name(prefix, &label);
            let (unit, divider) = unit_for(prefix);
            let value = read_sysfs_file(&file.path()).map(|v| v / divider as f32);
            let sensor_name = Some(SensorName {
                device_name: device_display_name.clone(),
                sensor_name: display_label,
            });
            let device_path_key = if let Some(dev) = &device_path {
                if dev.starts_with("DEADBEEF") {
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
                    device_path: device_path_key,
                },
                sensor_name,
                display_name: None,
                divider,
                unit,
                current_value: value,
            });
        }

        device_sensors.sort_by_cached_key(|s| s.get_display_name());
        out.extend(device_sensors);
    }

    out
}

/// Resolve the PCI ID (or platform name) for a hwmon directory.
///
/// Walks `…/device` symlink ancestors looking for the canonical
/// `DDDD:BB:dd.f` PCI address form; falls back to a `platform:` prefixed name
/// for non-PCI devices.
pub fn pci_id_from_path(hwmon_path: &Path) -> String {
    let device_path = hwmon_path.join("device");

    let Some(full_path) = std::fs::canonicalize(device_path).ok() else {
        return "None".to_string();
    };

    for component in full_path.ancestors() {
        if let Some(name_os) = component.file_name() {
            let name = name_os.to_string_lossy();
            if name.contains(':') && name.contains('.') && name.len() >= 7 {
                return name.into_owned();
            }
        }
        if component == Path::new("/sys/devices") {
            break;
        }
    }

    let name = std::fs::read_to_string(hwmon_path.join("name"))
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();

    format!("platform:{}", name)
}

/// Map an hwmon filename prefix (`temp1`, `fan2`, `in0`, `freq1`, …) to its
/// engineering unit and divider.
pub fn unit_for(prefix: &str) -> (Unit, usize) {
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

/// Produce a human-readable label from the sysfs `_label` file (or the prefix
/// itself when no label exists). Detects common substrings like "Control",
/// "Junction", "Edge", "VRAM", CCD/core die numbers, etc.
pub fn label_name(prefix: &str, label: &str) -> String {
    let lower_label = label.to_lowercase();
    let lower_prefix = prefix.to_lowercase();
    if lower_label.ends_with("ctl") || lower_label.ends_with("package id 0") {
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
    }
}

/// Classify an hwmon directory into a friendly device name.
///
/// Returns `(name, new_mem_idx, new_gfx_idx)`. `None` for the name means the
/// device should be skipped (e.g. ACPI thermal zones).
pub fn display_name(
    hwmon_path: &Path,
    pci_id_stripped: &str,
    gpu_names: &HashMap<String, String>,
    mem_idx: usize,
    gfx_idx: usize,
) -> (Option<String>, usize, usize) {
    let model_path = hwmon_path.join("device").join("model");

    if let Ok(model_name) = std::fs::read_to_string(model_path) {
        return (Some(model_name.trim().to_string()), mem_idx, gfx_idx);
    }

    if let Ok(generic_name) = std::fs::read_to_string(hwmon_path.join("name")) {
        let name = generic_name.trim();
        if name == "nvme" {
            return (Some("NVMe Storage Device".to_string()), mem_idx, gfx_idx);
        }
        if matches!(
            name,
            "k10temp" | "k8temp" | "coretemp" | "zenpower" | "zenpower3"
        ) {
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

/// Read a numeric sysfs file as f32.
pub fn read_sysfs_file(path: &Path) -> Option<f32> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: f32 = content.trim().parse().ok()?;
    Some(value)
}

/// Enumerate PWM fan headers (`pwm1..pwmN`) under each hwmon device.
pub fn enumerate_pwm_headers(gpu_names: &HashMap<String, String>) -> Vec<PwmHeader> {
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
        let (friendly, mi, gi) = display_name(&dir, &pci_id, gpu_names, mem_idx, gfx_idx);
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

/// Read the current duty cycle (0..=255) of a PWM header by its
/// `"{hwmonN}/pwm{n}"` identifier.
pub fn read_pwm_header(id: &str) -> Option<u8> {
    let path = Path::new("/sys/class/hwmon").join(id);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
}
