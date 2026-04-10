use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Image,
    Video,
    Color,
    Gif,
    Sensor,
    Doublegauge,
    Cooler,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SensorSourceConfig {
    Constant { value: f32 },
    Command { cmd: String },
    Hwmon {
        name: String,
        label: String,
        #[serde(default)]
        device_path: String,
    },
    #[serde(rename = "nvidia_gpu")]
    NvidiaGpu {
        #[serde(default)]
        gpu_index: u32,
    },
    #[serde(rename = "wireless_coolant")]
    WirelessCoolant {
        device_id: String,
    },
    #[serde(rename = "cpu_usage")]
    CpuUsage,
    #[serde(rename = "mem_usage")]
    MemUsage,
    #[serde(rename = "mem_used")]
    MemUsed,
    #[serde(rename = "mem_free")]
    MemFree,
}

impl Default for SensorSourceConfig {
    fn default() -> Self {
        SensorSourceConfig::CpuUsage
    }
}

impl SensorSourceConfig {
    pub fn to_sensor_source(&self) -> crate::sensors::SensorSource {
        match self {
            Self::Constant { value } => crate::sensors::SensorSource::Command {
                cmd: format!("echo {value}"),
            },
            Self::Command { cmd } => crate::sensors::SensorSource::Command { cmd: cmd.clone() },
            Self::Hwmon {
                name,
                label,
                device_path,
            } => crate::sensors::SensorSource::Hwmon {
                name: name.clone(),
                label: label.clone(),
                device_path: device_path.clone(),
            },
            Self::NvidiaGpu { gpu_index } => crate::sensors::SensorSource::NvidiaGpu {
                gpu_index: *gpu_index,
            },
            Self::WirelessCoolant { device_id } => crate::sensors::SensorSource::WirelessCoolant {
                device_id: device_id.clone(),
            },
            Self::CpuUsage => crate::sensors::SensorSource::CpuUsage,
            Self::MemUsage => crate::sensors::SensorSource::MemUsage,
            Self::MemUsed => crate::sensors::SensorSource::MemUsed,
            Self::MemFree => crate::sensors::SensorSource::MemFree,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SensorRange {
    pub max: Option<f32>,
    pub color: [u8; 3],
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SensorDescriptor {
    pub label: String,
    pub unit: String,
    pub source: SensorSourceConfig,
    #[serde(default = "default_text_color")]
    pub text_color: [u8; 3],
    #[serde(default = "default_background_color")]
    pub background_color: [u8; 3],
    #[serde(default = "default_gauge_background")]
    pub gauge_background_color: [u8; 3],
    #[serde(default = "default_ranges")]
    pub gauge_ranges: Vec<SensorRange>,
    #[serde(default = "default_update_ms")]
    pub update_interval_ms: u64,
    #[serde(default = "default_gauge_start_angle")]
    pub gauge_start_angle: f32,
    #[serde(default = "default_gauge_sweep_angle")]
    pub gauge_sweep_angle: f32,
    #[serde(default = "default_gauge_outer_radius")]
    pub gauge_outer_radius: f32,
    #[serde(default = "default_gauge_thickness")]
    pub gauge_thickness: f32,
    #[serde(default = "default_bar_corner_radius")]
    pub bar_corner_radius: f32,
    #[serde(default = "default_value_font_size")]
    pub value_font_size: f32,
    #[serde(default = "default_unit_font_size")]
    pub unit_font_size: f32,
    #[serde(default = "default_label_font_size")]
    pub label_font_size: f32,
    pub font_path: Option<PathBuf>,
    #[serde(default)]
    pub decimal_places: u8,
    #[serde(default)]
    pub value_offset: i32,
    #[serde(default = "default_unit_offset")]
    pub unit_offset: i32,
    #[serde(default = "default_label_offset")]
    pub label_offset: i32,
}

impl SensorDescriptor {
    pub fn validate(&self) -> anyhow::Result<()> {
        match &self.source {
            SensorSourceConfig::Constant { value } => {
                if !value.is_finite() {
                    anyhow::bail!("sensor constant value must be finite");
                }
                if *value < 0.0 || *value > 100.0 {
                    anyhow::bail!("sensor constant value must be between 0 and 100");
                }
            }
            SensorSourceConfig::Command { cmd } => {
                if cmd.trim().is_empty() {
                    anyhow::bail!("sensor command must not be empty");
                }
            }
            SensorSourceConfig::Hwmon { name, label, .. } => {
                if name.trim().is_empty() || label.trim().is_empty() {
                    anyhow::bail!("sensor hwmon name and label must not be empty");
                }
            }
            SensorSourceConfig::NvidiaGpu { .. } => {}
            SensorSourceConfig::WirelessCoolant { device_id } => {
                if device_id.trim().is_empty() {
                    anyhow::bail!("wireless coolant device_id must not be empty");
                }
            }
            SensorSourceConfig::CpuUsage
            | SensorSourceConfig::MemUsage
            | SensorSourceConfig::MemUsed
            | SensorSourceConfig::MemFree => {}
        }

        if self.update_interval_ms == 0 {
            anyhow::bail!("sensor update_interval_ms must be greater than zero");
        }

        if self.gauge_sweep_angle <= 0.0 || self.gauge_sweep_angle > 360.0 {
            anyhow::bail!("sensor gauge_sweep_angle must be within (0, 360] degree range");
        }

        if self.gauge_thickness <= 0.0 {
            anyhow::bail!("sensor gauge_thickness must be positive");
        }

        if self.gauge_outer_radius <= self.gauge_thickness + 5.0 {
            anyhow::bail!("sensor gauge_outer_radius must exceed gauge_thickness by at least 5");
        }

        if self.value_font_size <= 0.0 || self.unit_font_size <= 0.0 || self.label_font_size <= 0.0
        {
            anyhow::bail!("sensor font sizes must be greater than zero");
        }

        if self.bar_corner_radius < 0.0 {
            anyhow::bail!("sensor bar_corner_radius must be non-negative");
        }

        if self.decimal_places > 10 {
            anyhow::bail!("sensor decimal_places must be 10 or less");
        }

        if let Some(path) = &self.font_path {
            if !path.exists() {
                anyhow::bail!("sensor font_path '{}' does not exist", path.display());
            }
        }

        let mut last_max = -f32::INFINITY;
        for range in &self.gauge_ranges {
            if let Some(max) = range.max {
                if max < last_max {
                    anyhow::bail!("sensor gauge ranges must be sorted by max value");
                }
                if !(0.0..=100.0).contains(&max) {
                    anyhow::bail!("sensor gauge range max must be between 0 and 100");
                }
            }
            last_max = range.max.unwrap_or(100.0);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DoublegaugeDescriptor {
    #[serde(default)]
    pub header: String,

    #[serde(default)]
    pub gauge_1_min: i32,
    #[serde(default = "default_100")]
    pub gauge_1_max: i32,
    #[serde(default)]
    pub value_1_min: i32,
    #[serde(default = "default_100")]
    pub value_1_max: i32,
    #[serde(default)]
    pub display_value_1_min: i32,
    #[serde(default = "default_100")]
    pub display_value_1_max: i32,
    #[serde(default = "default_true")]
    pub clamp_1: bool,
    #[serde(default = "default_percent")]
    pub unit_1: String,
    #[serde(default = "default_n_a")]
    pub label_1: String,
    #[serde(default)]
    pub decimals_1: usize,

    #[serde(default)]
    pub gauge_2_min: i32,
    #[serde(default = "default_100")]
    pub gauge_2_max: i32,
    pub value_2_min: i32,
    #[serde(default = "default_100")]
    pub value_2_max: i32,
    pub display_value_2_min: i32,
    #[serde(default = "default_100")]
    pub display_value_2_max: i32,
    #[serde(default = "default_true")]
    pub clamp_2: bool,
    #[serde(default = "default_percent")]
    pub unit_2: String,
    #[serde(default = "default_n_a")]
    pub label_2: String,
    #[serde(default)]
    pub decimals_2: usize,
}

impl DoublegaugeDescriptor {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.gauge_1_max == self.gauge_1_min {
            anyhow::bail!("doublegauge gauge_1_min and gauge_1_max must differ");
        }
        if self.gauge_2_max == self.gauge_2_min {
            anyhow::bail!("doublegauge gauge_2_min and gauge_2_max must differ");
        }
        if self.value_1_max == self.value_1_min {
            anyhow::bail!("doublegauge value_1_min and value_1_max must differ");
        }
        if self.value_2_max == self.value_2_min {
            anyhow::bail!("doublegauge value_2_min and value_2_max must differ");
        }
        if self.decimals_1 > 10 || self.decimals_2 > 10 {
            anyhow::bail!("doublegauge decimals must be 10 or less");
        }
        Ok(())
    }
}

fn default_text_color() -> [u8; 3] {
    [255, 255, 255]
}

fn default_background_color() -> [u8; 3] {
    [0, 0, 0]
}

fn default_gauge_background() -> [u8; 3] {
    [60, 60, 60]
}

fn default_ranges() -> Vec<SensorRange> {
    vec![
        SensorRange {
            max: Some(50.0),
            color: [0, 200, 0],
        },
        SensorRange {
            max: Some(80.0),
            color: [220, 140, 0],
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
        },
    ]
}

fn default_update_ms() -> u64 {
    1_000
}

fn default_gauge_start_angle() -> f32 {
    90.0
}

fn default_gauge_sweep_angle() -> f32 {
    330.0
}

fn default_gauge_outer_radius() -> f32 {
    180.0
}

fn default_gauge_thickness() -> f32 {
    40.0
}

fn default_bar_corner_radius() -> f32 {
    0.0
}

fn default_value_font_size() -> f32 {
    72.0
}

fn default_unit_font_size() -> f32 {
    32.0
}

fn default_label_font_size() -> f32 {
    28.0
}

fn default_unit_offset() -> i32 {
    60
}

fn default_label_offset() -> i32 {
    -60
}

fn default_n_a() -> String {
    "N/A".to_string()
}

fn default_100() -> i32 {
    100
}

fn default_percent() -> String {
    "%".to_string()
}

fn default_true() -> bool {
    true
}
