//! Top-level sensor enumeration orchestrator.
//!
//! This module coordinates the themed submodules:
//! - [`hwmon`] walks `/sys/class/hwmon` (CPU temps, fan RPMs, voltages, PWM).
//! - [`gpu`] queries `nvidia-smi` and `/sys/class/drm` for NVIDIA/AMD GPUs.
//! - [`net_disk`] adds network interface and block-device rate sensors.
//!
//! [`enumerate_sensors`] is the single entry point used by the daemon and GUI.

use super::{PwmHeader, SensorInfo, SensorSource, Unit};

pub mod gpu;
pub mod hwmon;
pub mod net_disk;

// Re-export the public helpers that callers (daemon, GUI) depend on. These
// keep the old flat paths (`sensors::enumerate::xxx`) working.
pub use gpu::get_amd_gpu_names;
pub use hwmon::{display_name, label_name, pci_id_from_path, read_pwm_header, unit_for};

/// Enumerate every sensor the daemon knows about.
///
/// Order matters for the GUI's default ordering: built-in CPU/RAM sensors first,
/// then hwmon entries (sorted by display name), then NVIDIA, AMD, network, disk.
pub fn enumerate_sensors() -> Vec<SensorInfo> {
    let mut sensors = Vec::new();

    // Built-in synthetic sensors.
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

    // `/sys/class/hwmon` walk (CPU temps, fan RPMs, voltages, etc.).
    let gpu_names = gpu::get_amd_gpu_names();
    sensors.extend(hwmon::walk_hwmon(&gpu_names));

    // GPU enumeration.
    gpu::enumerate_nvidia(&mut sensors);
    gpu::enumerate_amd(&gpu_names, &mut sensors);

    // Network and disk rate sensors.
    net_disk::enumerate_network(&mut sensors);
    net_disk::enumerate_disk(&mut sensors);

    sensors
}

/// Enumerate PWM fan-header sysfs entries. Thin wrapper over
/// [`hwmon::enumerate_pwm_headers`] that pre-computes the AMD GPU name map.
pub fn enumerate_pwm_headers() -> Vec<PwmHeader> {
    let gpu_names = gpu::get_amd_gpu_names();
    hwmon::enumerate_pwm_headers(&gpu_names)
}
