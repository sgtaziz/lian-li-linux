use crate::aio::AioConfig;
use crate::fan::{FanConfig, FanCurve};
use crate::media::{DoublegaugeDescriptor, MediaType, SensorDescriptor, SensorSourceConfig};
use crate::rgb::RgbAppConfig;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LcdConfig {
    #[serde(default)]
    pub index: Option<usize>,
    pub serial: Option<String>,
    #[serde(rename = "type")]
    pub media_type: MediaType,
    pub path: Option<PathBuf>,
    pub fps: Option<f32>,
    // Polling interval for sensor-driven media types (Sensor, Doublegauge,
    // Cooler). `fps` stays scoped to Video/GIF. Unset → 1000ms.
    #[serde(default)]
    pub update_interval_ms: Option<u64>,
    pub rgb: Option<[u8; 3]>,
    #[serde(default)]
    pub orientation: f32,
    #[serde(default)]
    pub sensor: Option<SensorDescriptor>,
    // As most media types display sensor values, we store the selected sensors here. So if the media type switches, the sensor keeps the same.
    #[serde(default)]
    pub sensor_source_1: SensorSourceConfig,
    #[serde(default)]
    pub sensor_source_2: SensorSourceConfig,
    #[serde(default)]
    pub doublegauge: Option<DoublegaugeDescriptor>,
    #[serde(default)]
    pub template_id: Option<String>,
}

impl LcdConfig {
    pub fn device_id(&self) -> String {
        if let Some(serial) = &self.serial {
            format!("serial:{serial}")
        } else if let Some(index) = self.index {
            format!("index:{index}")
        } else {
            "unknown".to_string()
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.index.is_none() && self.serial.is_none() {
            bail!("device config requires either 'index' or 'serial' field");
        }

        let device_id = self.device_id();

        match self.media_type {
            MediaType::Image | MediaType::Video | MediaType::Gif => {
                let path = self
                    .path
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("LCD[{device_id}] requires a media path"))?;
                if !path.exists() {
                    bail!(
                        "LCD[{device_id}] media path '{}' does not exist",
                        path.display()
                    );
                }
            }
            MediaType::Color => {
                if self.rgb.is_none() {
                    bail!("LCD[{device_id}] color entry requires an 'rgb' field");
                }
            }
            MediaType::Sensor => {
                let descriptor = self.sensor.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "LCD[{device_id}] sensor configuration missing 'sensor' section"
                    )
                })?;
                descriptor.validate()?;
            }
            MediaType::Doublegauge | MediaType::Cooler => {}
            MediaType::Custom => {
                if self
                    .template_id
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    bail!("LCD[{device_id}] custom entry requires a 'template_id' field");
                }
            }
        }

        if let Some(fps) = self.fps {
            if fps <= 0.0 {
                bail!("LCD[{device_id}] fps must be positive");
            }
        }

        if let Some(ms) = self.update_interval_ms {
            if !(100..=10_000).contains(&ms) {
                bail!("LCD[{device_id}] update_interval_ms must be between 100 and 10000");
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HidDriver {
    Hidapi,
    Rusb,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_fps")]
    pub default_fps: f32,
    #[serde(default)]
    pub hid_driver: HidDriver,
    #[serde(default, alias = "devices")]
    pub lcds: Vec<LcdConfig>,
    #[serde(default)]
    pub fan_curves: Vec<FanCurve>,
    #[serde(default)]
    pub fans: Option<FanConfig>,
    #[serde(default)]
    pub rgb: Option<RgbAppConfig>,
    /// Per-AIO configuration keyed by device_id (e.g. "wireless:AA:BB:CC:DD:EE:FF").
    #[serde(default)]
    pub aio: HashMap<String, AioConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_fps: default_fps(),
            hid_driver: HidDriver::default(),
            lcds: Vec::new(),
            fan_curves: Vec::new(),
            fans: None,
            rgb: None,
            aio: HashMap::new(),
        }
    }
}

impl Default for HidDriver {
    fn default() -> Self {
        Self::Hidapi
    }
}

fn default_fps() -> f32 {
    30.0
}

impl AppConfig {
    /// Load and validate config. Returns the config and a list of non-fatal warnings
    /// (e.g. invalid LCD entries that were skipped).
    pub fn load(path: &Path) -> Result<(Self, Vec<String>)> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut cfg: AppConfig = serde_json::from_reader(reader)
            .with_context(|| format!("parsing {}", path.display()))?;

        let base_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        for device in &mut cfg.lcds {
            let identifier = if let Some(serial) = &device.serial {
                format!("serial:{serial}")
            } else if let Some(index) = device.index {
                format!("index:{index}")
            } else {
                warnings.push("LCD entry missing both 'index' and 'serial'".to_string());
                continue;
            };

            if !seen.insert(identifier.clone()) {
                warnings.push(format!("Duplicate LCD entry '{identifier}'"));
            }

            match device.media_type {
                MediaType::Doublegauge | MediaType::Cooler => {
                    device.media_type = MediaType::Custom;
                    device.template_id = None;
                    device.doublegauge = None;
                }
                _ => {}
            }

            if let Some(existing) = &device.path {
                if existing.is_relative() {
                    device.path = Some(base_dir.join(existing));
                }
            }

            if let Some(sensor) = &mut device.sensor {
                if let Some(font_path) = &sensor.font_path {
                    if font_path.is_relative() {
                        sensor.font_path = Some(base_dir.join(font_path));
                    }
                }
                // Legacy configs stored the sensor poll rate inside the
                // descriptor; promote it to the top-level field so Doublegauge
                // / Cooler pick it up too. Zero out the descriptor copy after
                // migration so future saves don't re-emit the stale value.
                if sensor.update_interval_ms != 0 {
                    if device.update_interval_ms.is_none() {
                        device.update_interval_ms = Some(sensor.update_interval_ms);
                    }
                    sensor.update_interval_ms = 0;
                }
            }

            if let Err(e) = device.validate() {
                warnings.push(format!("{e}"));
            }
        }

        if cfg.default_fps <= 0.0 {
            bail!("default_fps must be greater than zero");
        }

        // Normalize orientations to nearest 90°
        for device in &mut cfg.lcds {
            let normalized = (device.orientation % 360.0 + 360.0) % 360.0;
            let snapped = ((normalized + 45.0) / 90.0).floor() * 90.0;
            device.orientation = snapped % 360.0;
        }

        Ok((cfg, warnings))
    }
}

impl AppConfig {
    /// One-way migration: if a legacy `FanGroup` targets an AIO device and no
    /// `AioConfig` exists for that device_id yet, convert the FanGroup into an
    /// AioConfig (pump slot → pump_target_rpm, other slots → fan_speeds) and
    /// remove the FanGroup. Returns true if anything was migrated.
    pub fn migrate_aio_fangroup(&mut self, aio_device_id: &str) -> bool {
        if self.aio.contains_key(aio_device_id) {
            return false;
        }
        let Some(fans) = self.fans.as_mut() else {
            return false;
        };
        let Some(pos) = fans
            .speeds
            .iter()
            .position(|g| g.device_id.as_deref() == Some(aio_device_id))
        else {
            return false;
        };
        let group = fans.speeds.remove(pos);
        let mut aio = AioConfig::default();
        aio.pump_target_rpm = group.speeds[3].clone();
        aio.fan_speeds = group.speeds;
        self.aio.insert(aio_device_id.to_string(), aio);
        true
    }
}

pub type ConfigKey = String;

pub fn config_identity(cfg: &LcdConfig) -> ConfigKey {
    to_string(cfg).unwrap_or_else(|_| cfg.device_id())
}
