//! Sensor types and read pipeline.
//!
//! `SensorSource` is a serializable identifier persisted in config files.
//! `ResolvedSensor` is the runtime form (resolved sysfs path, NVIDIA index,
//! rate-counter state, etc.) used to actually read a value. The pipeline is:
//! enumerate → pick → resolve → read.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub mod enumerate;
pub mod picker;
pub mod read;
pub mod resolve;

pub use enumerate::{
    display_name, enumerate_pwm_headers, enumerate_sensors, label_name, pci_id_from_path,
    read_pwm_header,
};
pub use picker::{
    find_default_cpu_temp, find_default_gpu_temp, infer_sensor_category, pick_source_for_category,
};
pub use read::{get_mem_usage, read_sensor_value};
pub use resolve::{coolant_runtime_path, resolve_sensor, write_coolant_temp};

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

impl SensorName {
    pub fn get_device_name(&self) -> &str {
        &self.device_name
    }

    pub fn get_sensor_name(&self) -> &str {
        &self.sensor_name
    }
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

#[derive(Debug, Clone)]
pub struct PwmHeader {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
}
