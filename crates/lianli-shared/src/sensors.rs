use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::HashMap;

use crate::systeminfo::SysSensor;

/// SensorSource stores the information of a sensor in a way so that we can store it in a file, reboot, reload the file and are still able to find the sensor.
/// In order to actually read the sensor value the implemented way is to create a ResolvedSensor from SensorSource and use that.

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
    },
    Command {
        cmd: String,
    },
    WirelessCoolant {
        device_id: String,
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
    WO,
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let symbol = match self {
            Unit::C => "°C",
            Unit::RPM => "RPM",
            Unit::V => "mV",
            Unit::FREQ => "Mhz",
            Unit::SIZE => "MB",
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
        self.display_name
            .clone()
            .unwrap_or_else(|| {
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

#[derive(Debug, Clone)]
pub enum ResolvedSensor {
    SysfsFile { 
        path: PathBuf, 
        device_path: String, // from SensorSource, used in order to define what to read from the sysfsfile: basic hwmon entries are easy, just read the file, it contains a number. But the file /proc/meminfo contains more than just a number
        divider: usize,
    },
    NvidiaGpu(u32),
    ShellCommand(String),
    /// Runtime file written by daemon, contains plain °C value (not millidegrees).
    RuntimeFile(PathBuf),
}

pub fn enumerate_sensors() -> Vec<SensorInfo> {
    let mut sensors = Vec::new();
    
    let mut mem_idx: usize = 0;
    let mut gfx_idx: usize = 0;
    let mut display_name: String;

    let gpu_names = get_amd_gpu_names();

    // First of all, add some default sensors:
    // CPU Usage
    sensors.push(SensorInfo {
        source: SensorSource::Hwmon {
            name: "CPU".to_string(),
            label: "Usage".to_string(),
            device_path: "direct:cpu_usage".to_string(),
        },
        sensor_name: Some(SensorName { device_name: "CPU".to_string(), sensor_name: "Usage".to_string() }),
        display_name: None,
        divider: 100,
        unit: Unit::PERCENT,
        current_value: Some(0.0),
    });

    // RAM Usage in percent
    sensors.push(SensorInfo {
        source: SensorSource::Hwmon {
            name: "RAM".to_string(),
            label: "Usage".to_string(),
            device_path: "direct:mem_usage".to_string(),
        },
        sensor_name: Some(SensorName { device_name: "RAM".to_string(), sensor_name:"Usage".to_string() }),
        display_name: None,
        divider: 1,
        unit: Unit::PERCENT,
        current_value: Some(0.0),
    });

    // RAM Used in MB
    sensors.push(SensorInfo {
        source: SensorSource::Hwmon {
            name: "RAM".to_string(),
            label: "Used".to_string(),
            device_path: "direct:mem_used".to_string(),
        },
        sensor_name: Some(SensorName { device_name: "RAM".to_string(), sensor_name:"Used".to_string() }),
        display_name: None,
        divider: 1024 * 1024,
        unit: Unit::SIZE,
        current_value: Some(0.0),
    });

    // RAM Free in MB
    sensors.push(SensorInfo {
        source: SensorSource::Hwmon {
            name: "RAM".to_string(),
            label: "Free".to_string(),
            device_path: "direct:mem_free".to_string(),
        },
        sensor_name: Some(SensorName { device_name: "RAM".to_string(), sensor_name:"Free".to_string() }),
        display_name: None,
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

            (display_name, mem_idx, gfx_idx) = get_display_name(&path, &pci_id_stripped, &gpu_names, mem_idx, gfx_idx);

            if display_name == "ignore" {
                // This element is to ignore (for example ACPI thermal zone is a quite unreliable temperature sensor, so omitting it is recommended)
                // I know, this is not rustic, but it works for me!!
                continue;
            }


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
                        let value = read_sysfs_file(&file.path());
                        let sensor_name = Some(SensorName { device_name: display_name.clone(), sensor_name: display_label });
                        let device_path = if let Some(dev) = &device_path {
                            if dev.starts_with("DEADBEEF") { // virtual devices (for example my motherboard from Gigabyte links to "DEADBEEF-2001-0000-00A0-C90629100000")
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
        .args(["--query-gpu=index,name,temperature.gpu", "--format=csv,noheader,nounits"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(", ").collect();
                if parts.len() >= 3 {
                    let gpu_index: u32 = parts[0].trim().parse().unwrap_or(0);
                    let gpu_name = parts[1].trim();
                    let temp: Option<f32> = parts[2].trim().parse().ok();

                    sensors.push(SensorInfo {
                        source: SensorSource::NvidiaGpu { gpu_index },
                        sensor_name: None,
                        display_name: Some(format!("{gpu_name} (GPU)")),
                        current_value: temp,
                        unit: Unit::C,
                        divider: 1,
                    });
                }
            }
        }
    }
    sensors
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
) -> (String, usize, usize) {
    // First of all, check for file device/model and return that in case it exists and contains something
    // This will display for example the type and name of the nvme-SSD
    let model_path = hwmon_path.join("device").join("model");

    if let Ok(model_name) = std::fs::read_to_string(model_path) {
        return (model_name.trim().to_string(), mem_idx, gfx_idx);
    }

    // Fallback: Read the normal 'name' file
    // Path: /sys/class/hwmon/hwmonX/name
    if let Ok(generic_name) = std::fs::read_to_string(hwmon_path.join("name")) {
        let name = generic_name.trim();
        // If the name is only "nvme", then make it a little prettier
        if name == "nvme" {
            return ("NVMe Storage Device".to_string(), mem_idx, gfx_idx);
        }
        // AMD processors have k10temp, Intel have coretemp
        if name == "k10temp" || name == "coretemp" {
            return ("CPU".to_string(), mem_idx, gfx_idx);
        }
        // AMD gpus (internal or external) are named amdgpu
        if name == "amdgpu" {
            if gfx_idx > 0 {
                // The first graphics card is the main card. If there is a second graphics card, then it's the internal graphics chip, which is of no interest as not used.
                // Well, if somebody has more than one PCI graphics card, then we won't display the second one, I know, but in most cases the second one is just the internal graphics chip.
                return ("ignore".to_string(), mem_idx, gfx_idx);
            }
            if let Some(name) = gpu_names.get(pci_id_stripped) {
                return (name.clone(), mem_idx, gfx_idx + 1);
            }
        }
        // NVidia gpus either don't appear at all, or appear as nouveau (if that driver is used)
        if name == "nouveau" {
            return ("NVidia GPU".to_string(), mem_idx, gfx_idx + 1);
        }
        let common_drivers = ["nct", "it8", "f71", "gigabyte_wmi", "w83"];
        if common_drivers.iter().any(|&d| name.starts_with(d)) {
            return ("Motherboard".to_string(), mem_idx, gfx_idx);
        }
        if name.starts_with("spd") {
            return (format!("DDR5 RAM Module {}",mem_idx+1), mem_idx + 1, gfx_idx);
        }
        if name.starts_with("ee1004") {
            return (format!("DDR4 RAM Module {}",mem_idx+1), mem_idx + 1, gfx_idx);
        }
        if name.starts_with("jc42") {
            return (format!("DDR3/ECC RAM Module {}",mem_idx+1), mem_idx + 1, gfx_idx);
        }
        if name == "acpitz" {
            // Notoriously inaccurate sensor: ACPI Thermal Zone, best to just ignore it...
            return ("ignore".to_string(), mem_idx, gfx_idx);
        }

        (name.to_string(), mem_idx, gfx_idx)
    } else {
        ("Unknown Device".to_string(), mem_idx, gfx_idx)
    }
}

pub fn resolve_sensor(source: &SensorSource, divider: usize) -> Option<ResolvedSensor> {
    match source {
        SensorSource::Hwmon {
            name: _,
            label,
            device_path,
        } => {
            if device_path == "direct:cpu_usage" {
                return Some(ResolvedSensor::SysfsFile { path: Path::new("/sys/class/hwmon").to_path_buf(), device_path: device_path.clone(), divider });
            } else if device_path=="direct:mem_usage" {
                return Some(ResolvedSensor::SysfsFile { path: Path::new("/proc/meminfo").to_path_buf(), device_path: device_path.clone(), divider });
            } else if device_path=="direct:mem_used" {
                return Some(ResolvedSensor::SysfsFile { path: Path::new("/proc/meminfo").to_path_buf(), device_path: device_path.clone(), divider });
            } else if device_path=="direct:mem_free" {
                return Some(ResolvedSensor::SysfsFile { path: Path::new("/proc/meminfo").to_path_buf(), device_path: device_path.clone(), divider });
            }
            let hwmon_dir = Path::new("/sys/class/hwmon");
            let entries = std::fs::read_dir(hwmon_dir).ok()?;

            for entry in entries.flatten() {
                let path = entry.path();

                let device_path_symlink = std::fs::read_link(path.join("device"))
                    .ok()
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));
                
                let curr_device_path = if let Some(dev) = &device_path_symlink {
                    if dev.starts_with("DEADBEEF") { // virtual devices like my motherboard links to "DEADBEEF-2001-0000-00A0-C90629100000"
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

                // Search *_input files for matching label
                if let Ok(files) = std::fs::read_dir(&path) {
                    for file in files.flatten() {
                        let fname = file.file_name().to_string_lossy().to_string();
                        if fname.ends_with("_input") {
                            let prefix = fname.strip_suffix("_input").unwrap();
                            if prefix == label {
                                return Some(ResolvedSensor::SysfsFile { path: file.path(), device_path: device_path.clone(), divider });
                            }
                        }
                    }
                }
            }
            None
        }
        SensorSource::NvidiaGpu { gpu_index } => Some(ResolvedSensor::NvidiaGpu(*gpu_index)),
        SensorSource::Command { cmd } => Some(ResolvedSensor::ShellCommand(cmd.clone())),
        SensorSource::WirelessCoolant { device_id } => {
            let path = coolant_runtime_path(device_id);
            if path.exists() {
                Some(ResolvedSensor::RuntimeFile(path))
            } else {
                None
            }
        }
    }
}

pub fn read_sensor_value(resolved: &ResolvedSensor) -> anyhow::Result<f32> {
    match resolved {
        ResolvedSensor::SysfsFile { path, device_path, divider } => {
            if device_path == "direct:cpu_usage" {
                let ret = SysSensor::get_cpu_usage();
                return Ok((ret as f32) / (*divider as f32));
            }
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;

            if device_path == "direct:mem_usage" {
                return Ok(get_mem_usage(&content));
            }
            if device_path == "direct:mem_used" {
                let total = extract_mem_value(&content, "MemTotal:").unwrap_or(0.0);
                let avail = extract_mem_value(&content, "MemAvailable:").unwrap_or(0.0);
                return Ok((total - avail) / (*divider as f32));
            }
            if device_path == "direct:mem_free" {
                return Ok(extract_mem_value(&content, "MemAvailable:").unwrap_or(0.0) / (*divider as f32));
            }
            let raw_value: f32 = content
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
            Ok(raw_value / (*divider as f32))
        }
        ResolvedSensor::NvidiaGpu(index) => {
            let output = Command::new("nvidia-smi")
                .args([
                    "--query-gpu=temperature.gpu",
                    "--format=csv,noheader,nounits",
                    "-i",
                    &index.to_string(),
                ])
                .output()
                .map_err(|e| anyhow::anyhow!("nvidia-smi: {e}"))?;
            if !output.status.success() {
                anyhow::bail!("nvidia-smi failed");
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let temp: f32 = stdout
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("parsing nvidia-smi output: {e}"))?;
            Ok(temp)
        }
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
    }
}

/// Runtime path for a wireless coolant temperature file.
pub fn coolant_runtime_path(device_id: &str) -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());
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
        return 100.0 - avail * 100.0 / total;
    }
    0.0
}

fn extract_mem_value(input: &str, target: &str) -> Option<f32> {
    let line = input.lines().find(|l| l.starts_with(target))?;
    let parts: Vec<&str> = line.split_whitespace().collect();
    parts.get(1)?.parse::<f32>().ok()
}
