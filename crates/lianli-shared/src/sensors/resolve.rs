use super::enumerate::{get_pci_id_from_path, get_unit};
use super::{RateState, ResolvedSensor, SensorSource};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const COOLANT_MAX_AGE: Duration = Duration::from_secs(10);
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
            // Wired AIO telemetry creates this runtime file shortly after the
            // daemon has initialized its media assets.  Keep the path resolved
            // even when the first value has not been published yet so custom
            // LCD widgets retry the read and start updating as soon as it is.
            Some(ResolvedSensor::RuntimeFile {
                path,
                max_age: Some(COOLANT_MAX_AGE),
            })
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

/// Return the per-user runtime directory without falling back to a shared,
/// symlinkable temporary path.
fn runtime_base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })))
        .join("lianli-linux")
}

/// Derive a non-reversible, single-component filename for a device ID.
fn coolant_runtime_path_in(runtime_dir: &Path, device_id: &str) -> PathBuf {
    let digest = Sha256::digest(device_id.as_bytes());
    runtime_dir.join(format!("coolant-{}.txt", hex::encode(&digest[..12])))
}

/// Runtime path for a coolant temperature file. Device identifiers are hashed
/// so serials cannot escape the private runtime directory or collide after
/// lossy filename sanitization.
pub fn coolant_runtime_path(device_id: &str) -> PathBuf {
    coolant_runtime_path_in(&runtime_base_dir(), device_id)
}

/// Publish one validated sample below an explicitly supplied runtime directory.
fn write_coolant_temp_at(runtime_dir: &Path, device_id: &str, temp_c: f32) -> Result<()> {
    if !temp_c.is_finite() || !(0.0..=100.0).contains(&temp_c) {
        anyhow::bail!("refusing invalid coolant temperature {temp_c}");
    }

    std::fs::create_dir_all(runtime_dir)
        .with_context(|| format!("creating runtime directory {}", runtime_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(runtime_dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("securing runtime directory {}", runtime_dir.display()))?;
    }

    let path = coolant_runtime_path_in(runtime_dir, device_id);
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = runtime_dir.join(format!(".coolant-{}-{sequence}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("creating temporary sensor file {}", temp_path.display()))?;
        write!(file, "{temp_c:.1}")
            .with_context(|| format!("writing temporary sensor file {}", temp_path.display()))?;
        file.flush()
            .with_context(|| format!("flushing temporary sensor file {}", temp_path.display()))?;
        std::fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "publishing coolant sensor {} as {}",
                temp_path.display(),
                path.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

/// Atomically publish a coolant temperature value for LCD widgets and curves.
pub fn write_coolant_temp(device_id: &str, temp_c: f32) -> Result<()> {
    write_coolant_temp_at(&runtime_base_dir(), device_id, temp_c)
}

/// Read a fresh coolant sample, rejecting files older than the safety window.
pub fn read_coolant_temp(device_id: &str) -> Result<f32> {
    super::read::read_sensor_value(&ResolvedSensor::RuntimeFile {
        path: coolant_runtime_path(device_id),
        max_age: Some(COOLANT_MAX_AGE),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lianli-shared-{name}-{}-{}",
            std::process::id(),
            TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn runtime_path_never_contains_device_path_components() {
        let root = PathBuf::from("/run/user/1000/lianli-linux");
        let path = coolant_runtime_path_in(&root, "../../other/\u{2603}:device");
        assert_eq!(path.parent(), Some(root.as_path()));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("coolant-"));
        assert!(!path.to_string_lossy().contains("other"));
    }

    #[test]
    fn atomic_write_publishes_valid_temperature() {
        let dir = test_dir("write");
        write_coolant_temp_at(&dir, "hid:test", 35.7).unwrap();
        let resolved = ResolvedSensor::RuntimeFile {
            path: coolant_runtime_path_in(&dir, "hid:test"),
            max_age: Some(COOLANT_MAX_AGE),
        };
        let value = super::super::read::read_sensor_value(&resolved).unwrap();
        assert!((value - 35.7).abs() < 0.01);
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expired_runtime_sample_is_rejected() {
        let dir = test_dir("stale");
        write_coolant_temp_at(&dir, "hid:test", 35.7).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        let resolved = ResolvedSensor::RuntimeFile {
            path: coolant_runtime_path_in(&dir, "hid:test"),
            max_age: Some(Duration::ZERO),
        };
        assert!(super::super::read::read_sensor_value(&resolved).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_values_are_not_published() {
        let dir = test_dir("invalid");
        for value in [f32::NAN, f32::INFINITY, -1.0, 100.1] {
            assert!(write_coolant_temp_at(&dir, "hid:test", value).is_err());
        }
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_publish_replaces_symlink_without_following_it() {
        use std::os::unix::fs::symlink;
        let dir = test_dir("symlink");
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim");
        std::fs::write(&victim, "unchanged").unwrap();
        let path = coolant_runtime_path_in(&dir, "hid:test");
        symlink(&victim, &path).unwrap();

        write_coolant_temp_at(&dir, "hid:test", 42.0).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "42.0");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
