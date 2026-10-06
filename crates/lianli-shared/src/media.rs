use crate::template::ImageFit;
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
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum SensorSourceConfig {
    Constant {
        value: f32,
    },
    Command {
        cmd: String,
    },
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
        #[serde(default)]
        metric: crate::sensors::NvidiaMetric,
    },
    #[serde(rename = "amd_gpu_usage")]
    AmdGpuUsage {
        #[serde(default)]
        card_index: u32,
    },
    #[serde(rename = "wireless_coolant")]
    WirelessCoolant {
        device_id: String,
    },
    #[serde(rename = "cpu_usage")]
    #[default]
    CpuUsage,
    #[serde(rename = "mem_usage")]
    MemUsage,
    #[serde(rename = "mem_used")]
    MemUsed,
    #[serde(rename = "mem_free")]
    MemFree,
    #[serde(rename = "network_rx")]
    NetworkRx {
        iface: String,
    },
    #[serde(rename = "network_tx")]
    NetworkTx {
        iface: String,
    },
    #[serde(rename = "disk_read")]
    DiskRead {
        device: String,
    },
    #[serde(rename = "disk_write")]
    DiskWrite {
        device: String,
    },
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
            Self::NvidiaGpu { gpu_index, metric } => crate::sensors::SensorSource::NvidiaGpu {
                gpu_index: *gpu_index,
                metric: *metric,
            },
            Self::AmdGpuUsage { card_index } => crate::sensors::SensorSource::AmdGpuUsage {
                card_index: *card_index,
            },
            Self::WirelessCoolant { device_id } => crate::sensors::SensorSource::WirelessCoolant {
                device_id: device_id.clone(),
            },
            Self::CpuUsage => crate::sensors::SensorSource::CpuUsage,
            Self::MemUsage => crate::sensors::SensorSource::MemUsage,
            Self::MemUsed => crate::sensors::SensorSource::MemUsed,
            Self::MemFree => crate::sensors::SensorSource::MemFree,
            Self::NetworkRx { iface } => crate::sensors::SensorSource::NetworkRate {
                iface: iface.clone(),
                direction: crate::sensors::NetDirection::Rx,
            },
            Self::NetworkTx { iface } => crate::sensors::SensorSource::NetworkRate {
                iface: iface.clone(),
                direction: crate::sensors::NetDirection::Tx,
            },
            Self::DiskRead { device } => crate::sensors::SensorSource::DiskRate {
                device: device.clone(),
                direction: crate::sensors::DiskDirection::Read,
            },
            Self::DiskWrite { device } => crate::sensors::SensorSource::DiskRate {
                device: device.clone(),
                direction: crate::sensors::DiskDirection::Write,
            },
        }
    }
}

/// How image, GIF and video sources are placed on an LCD. Zoom and offsets
/// select a region of the source; `fit` maps that region onto the screen.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct MediaFraming {
    pub fit: ImageFit,
    pub zoom: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for MediaFraming {
    fn default() -> Self {
        Self {
            fit: ImageFit::Stretch,
            zoom: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl MediaFraming {
    pub const MAX_ZOOM: f32 = 8.0;

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn sanitized(self) -> Self {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        Self {
            fit: self.fit,
            zoom: finite(self.zoom, 1.0).clamp(1.0, Self::MAX_ZOOM),
            offset_x: finite(self.offset_x, 0.0).clamp(-1.0, 1.0),
            offset_y: finite(self.offset_y, 0.0).clamp(-1.0, 1.0),
        }
    }

    pub fn source_crop(&self, source: (u32, u32), target: (u32, u32)) -> CropRect {
        let framing = self.sanitized();
        let (sw, sh) = (source.0.max(1) as f64, source.1.max(1) as f64);
        let (tw, th) = (target.0.max(1) as f64, target.1.max(1) as f64);
        let (base_w, base_h) = if framing.fit == ImageFit::Cover {
            (sw.min(sh * tw / th), sh.min(sw * th / tw))
        } else {
            (sw, sh)
        };
        let zoom = framing.zoom as f64;
        let width = (base_w / zoom).round().clamp(1.0, sw);
        let height = (base_h / zoom).round().clamp(1.0, sh);
        let x = ((sw - width) / 2.0 * (1.0 + framing.offset_x as f64))
            .round()
            .clamp(0.0, sw - width);
        let y = ((sh - height) / 2.0 * (1.0 + framing.offset_y as f64))
            .round()
            .clamp(0.0, sh - height);
        CropRect {
            x: x as u32,
            y: y as u32,
            width: width as u32,
            height: height as u32,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SensorRange {
    pub max: Option<f32>,
    pub color: [u8; 3],
    #[serde(default = "default_sensor_range_alpha")]
    pub alpha: u8,
}

fn default_sensor_range_alpha() -> u8 {
    255
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
    // Legacy: pre-0.3.3 configs stored the poll rate here. Kept solely so
    // `Config::load` can promote it to `LcdConfig::update_interval_ms` during
    // migration. New code should not read or write this field.
    #[serde(default)]
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
        self.validate_settings()?;
        if let Some(path) = &self.font_path {
            if !path.exists() {
                anyhow::bail!("sensor font_path '{}' does not exist", path.display());
            }
        }
        Ok(())
    }

    pub fn validate_settings(&self) -> anyhow::Result<()> {
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
            SensorSourceConfig::AmdGpuUsage { .. } => {}
            SensorSourceConfig::WirelessCoolant { device_id } => {
                if device_id.trim().is_empty() {
                    anyhow::bail!("wireless coolant device_id must not be empty");
                }
            }
            SensorSourceConfig::CpuUsage
            | SensorSourceConfig::MemUsage
            | SensorSourceConfig::MemUsed
            | SensorSourceConfig::MemFree => {}
            SensorSourceConfig::NetworkRx { iface } | SensorSourceConfig::NetworkTx { iface } => {
                if iface.trim().is_empty() {
                    anyhow::bail!("network sensor iface must not be empty");
                }
            }
            SensorSourceConfig::DiskRead { device } | SensorSourceConfig::DiskWrite { device } => {
                if device.trim().is_empty() {
                    anyhow::bail!("disk sensor device must not be empty");
                }
            }
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
    #[serde(default)]
    pub value_2_min: i32,
    #[serde(default = "default_100")]
    pub value_2_max: i32,
    #[serde(default)]
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
            alpha: 255,
        },
        SensorRange {
            max: Some(80.0),
            color: [220, 140, 0],
            alpha: 255,
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
            alpha: 255,
        },
    ]
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

#[cfg(test)]
mod framing_tests {
    use super::{CropRect, MediaFraming};
    use crate::template::ImageFit;

    fn framing(fit: ImageFit, zoom: f32, offset_x: f32, offset_y: f32) -> MediaFraming {
        MediaFraming {
            fit,
            zoom,
            offset_x,
            offset_y,
        }
    }

    #[test]
    fn default_framing_shows_the_whole_source() {
        let crop = MediaFraming::default().source_crop((1920, 1080), (400, 400));
        assert_eq!(
            crop,
            CropRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn cover_crops_the_source_to_the_screen_aspect() {
        let centered = framing(ImageFit::Cover, 1.0, 0.0, 0.0).source_crop((1600, 900), (400, 400));
        assert_eq!(
            (centered.x, centered.width, centered.height),
            (350, 900, 900)
        );
        let left = framing(ImageFit::Cover, 1.0, -1.0, 0.0).source_crop((1600, 900), (400, 400));
        assert_eq!(left.x, 0);
    }

    #[test]
    fn zoom_and_offsets_select_a_region_within_bounds() {
        let crop = framing(ImageFit::Stretch, 2.0, 1.0, -1.0).source_crop((1000, 500), (400, 400));
        assert_eq!(
            crop,
            CropRect {
                x: 500,
                y: 0,
                width: 500,
                height: 250
            }
        );
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let crop = framing(ImageFit::Contain, f32::NAN, 9.0, f32::INFINITY)
            .source_crop((800, 600), (400, 400));
        assert_eq!((crop.x, crop.y, crop.width, crop.height), (0, 0, 800, 600));
        let zoomed =
            framing(ImageFit::Stretch, 100.0, 0.0, 0.0).source_crop((800, 800), (400, 400));
        assert_eq!(zoomed.width, 100);
    }

    #[test]
    fn lcd_entries_without_framing_keep_their_serialized_form() {
        let legacy = serde_json::json!({"serial": "lcd", "type": "image", "path": "/a.png", "fps": null, "rgb": null});
        let config: crate::config::LcdConfig = serde_json::from_value(legacy).unwrap();
        assert!(config.framing.is_default());
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("framing")
            .is_none());
        let mut framed = config.clone();
        framed.framing.fit = ImageFit::Cover;
        let encoded = serde_json::to_value(&framed).unwrap();
        assert_eq!(encoded["framing"]["fit"], "cover");
        let decoded: crate::config::LcdConfig = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.framing.fit, ImageFit::Cover);
    }
}
