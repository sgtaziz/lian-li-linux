//! Network and disk rate sensors sourced from `/proc/net/dev` and
//! `/sys/block`.

use crate::sensors::{DiskDirection, NetDirection, SensorInfo, SensorSource, Unit};

/// Emit Rx/Tx rate sensors for every non-loopback network interface.
pub fn enumerate_network(sensors: &mut Vec<SensorInfo>) {
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

/// Emit read/write rate sensors for every physical block device.
///
/// Skips pseudo-devices: `loop*`, `ram*`, `dm-*`, `zram*`, `sr*` (optical).
pub fn enumerate_disk(sensors: &mut Vec<SensorInfo>) {
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
