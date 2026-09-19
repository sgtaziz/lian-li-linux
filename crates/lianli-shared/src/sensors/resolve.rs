use super::enumerate::{pci_id_from_path, unit_for};
use super::{RateState, ResolvedSensor, SensorSource};
use std::io::Write;
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
        } => resolve_hwmon(Path::new("/sys/class/hwmon"), name, label, device_path),
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

fn resolve_hwmon(
    root: &Path,
    name: &str,
    label: &str,
    device_path: &str,
) -> Option<ResolvedSensor> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
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

            let curr_device_path = match &device_path_symlink {
                Some(dev) if !dev.starts_with("DEADBEEF") => dev.to_string(),
                _ => pci_id_from_path(&path),
            };

            if curr_device_path != device_path {
                continue;
            }
        }

        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let fname = file.file_name().to_string_lossy().to_string();
            let Some(prefix) = fname.strip_suffix("_input") else {
                continue;
            };
            // Older configurations store the human-readable label.
            let matches_label = prefix == label
                || std::fs::read_to_string(path.join(format!("{prefix}_label")))
                    .is_ok_and(|value| value.trim() == label);
            if matches_label {
                return Some(ResolvedSensor::SysfsFile {
                    path: file.path(),
                    // Startup enumeration may predate this sensor, so its divider can be missing.
                    divider: unit_for(prefix).1,
                });
            }
        }
    }
    None
}

/// Runtime path for a wireless coolant temperature file.
pub fn coolant_runtime_path(device_id: &str) -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let sanitized = device_id.replace(':', "-");
    PathBuf::from(format!("{runtime_dir}/lianli-coolant-{sanitized}"))
}

/// Write a coolant temperature value to the runtime file.
pub fn write_coolant_temp(device_id: &str, temp_c: f32) {
    let _ = write_coolant_reading(
        device_id,
        super::SensorReading {
            value: temp_c,
            observed_at: std::time::Instant::now(),
        },
    );
}

pub fn write_coolant_reading(device_id: &str, reading: super::SensorReading) -> anyhow::Result<()> {
    write_runtime_reading(&coolant_runtime_path(device_id), reading)
}

fn write_runtime_reading(path: &Path, reading: super::SensorReading) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing coolant runtime directory"))?;
    let modified = std::time::SystemTime::now()
        .checked_sub(reading.observed_at.elapsed())
        .ok_or_else(|| anyhow::anyhow!("Invalid coolant observation time"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    write!(file, "{}", reading.value)?;
    file.as_file().set_modified(modified)?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod coolant_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn coolant_reader_rejects_nonregular_or_oversized_runtime_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coolant");
        let resolved = ResolvedSensor::RuntimeFile(path.clone());
        std::fs::write(&path, "1".repeat(65)).unwrap();
        assert!(super::super::read_sensor_reading(&resolved).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(directory.path(), &path).unwrap();
        assert!(super::super::read_sensor_reading(&resolved).is_err());
        assert!(
            super::super::read_sensor_reading(&ResolvedSensor::RuntimeFile(
                directory.path().to_owned()
            ))
            .is_err()
        );
    }

    #[test]
    fn republishing_coolant_preserves_age_and_plain_numeric_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("coolant");
        let observed_at = Instant::now() - Duration::from_secs(10);
        let reading = super::super::SensorReading {
            value: 42.0,
            observed_at,
        };
        for _ in 0..2 {
            write_runtime_reading(&path, reading).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "42");
            let resolved = ResolvedSensor::RuntimeFile(path.clone());
            let sample = super::super::read_sensor_reading(&resolved).unwrap();
            assert_eq!(sample.value, 42.0);
            assert!(sample.observed_at.elapsed() >= Duration::from_secs(9));
            assert!(super::super::read_sensor_value(&resolved).is_err());
        }
        write_runtime_reading(
            &path,
            super::super::SensorReading {
                value: 50.0,
                observed_at: Instant::now(),
            },
        )
        .unwrap();
        assert_eq!(
            super::super::read_sensor_value(&ResolvedSensor::RuntimeFile(path)).unwrap(),
            50.0
        );
    }
}

#[cfg(test)]
mod hwmon_tests {
    use super::*;

    fn fake_hwmon() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let chip = root.path().join("hwmon3");
        std::fs::create_dir(&chip).unwrap();
        std::fs::write(chip.join("name"), "amdgpu\n").unwrap();
        std::fs::write(chip.join("temp1_input"), "45000\n").unwrap();
        std::fs::write(chip.join("temp1_label"), "edge\n").unwrap();
        std::fs::write(chip.join("fan1_input"), "1200\n").unwrap();
        root
    }

    fn divider_of(resolved: Option<ResolvedSensor>) -> usize {
        match resolved.expect("sensor resolves") {
            ResolvedSensor::SysfsFile { divider, .. } => divider,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn hwmon_divider_derives_from_attribute_prefix() {
        let root = fake_hwmon();
        assert_eq!(
            divider_of(resolve_hwmon(root.path(), "amdgpu", "temp1", "")),
            1000
        );
        assert_eq!(
            divider_of(resolve_hwmon(root.path(), "amdgpu", "edge", "")),
            1000
        );
        assert_eq!(
            divider_of(resolve_hwmon(root.path(), "amdgpu", "fan1", "")),
            1
        );
    }

    #[test]
    fn hwmon_resolves_once_the_chip_appears() {
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_hwmon(root.path(), "amdgpu", "temp1", "").is_none());
        let chip = root.path().join("hwmon0");
        std::fs::create_dir(&chip).unwrap();
        std::fs::write(chip.join("name"), "amdgpu\n").unwrap();
        std::fs::write(chip.join("temp1_input"), "45000\n").unwrap();
        let resolved = resolve_hwmon(root.path(), "amdgpu", "temp1", "").unwrap();
        assert_eq!(super::super::read_sensor_value(&resolved).unwrap(), 45.0);
    }

    #[test]
    fn hwmon_missing_label_does_not_match_empty_config_label() {
        let root = fake_hwmon();
        assert!(resolve_hwmon(root.path(), "amdgpu", "", "").is_none());
    }
}
