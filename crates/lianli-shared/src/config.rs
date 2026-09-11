use crate::aio::AioConfig;
use crate::device_id::DeviceFamily;
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
    #[serde(default)]
    pub smooth_edges: Option<bool>,
    #[serde(default)]
    pub custom_h264: Option<bool>,
    #[serde(default)]
    pub aio_512_frame: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
}

impl LcdConfig {
    pub fn brightness(&self) -> u8 {
        self.brightness.unwrap_or(100).min(100)
    }

    pub fn smooth_edges(&self) -> bool {
        self.smooth_edges.unwrap_or(false)
    }

    pub fn aio_512_frame(&self) -> bool {
        self.aio_512_frame.unwrap_or(true)
    }

    /// HydroShift LCD firmware mishandles 512-byte HID frames (#137).
    pub fn aio_512_frame_for(&self, family: DeviceFamily) -> bool {
        self.aio_512_frame
            .unwrap_or(!matches!(family, DeviceFamily::HydroShiftLcd))
    }

    pub fn custom_h264(&self) -> bool {
        self.custom_h264.unwrap_or(true)
    }

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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Ene6k77DeviceConfig {
    #[serde(default)]
    pub fan_quantities: HashMap<u8, u8>,
}

fn default_threshold() -> u8 {
    80
}

fn default_cpu_alert_color() -> [u8; 3] {
    [255, 0, 0]
}

fn default_gpu_alert_color() -> [u8; 3] {
    [0, 0, 255]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThermalAlertSourceSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_threshold")]
    pub threshold: u8,
    #[serde(default = "default_cpu_alert_color")]
    pub alert_color: [u8; 3],
}

impl Default for ThermalAlertSourceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_threshold(),
            alert_color: default_cpu_alert_color(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ThermalAlertSettings {
    #[serde(default)]
    pub cpu: ThermalAlertSourceSettings,
    #[serde(default = "default_gpu_settings")]
    pub gpu: ThermalAlertSourceSettings,
}

fn default_gpu_settings() -> ThermalAlertSourceSettings {
    ThermalAlertSourceSettings {
        enabled: false,
        threshold: 80,
        alert_color: default_gpu_alert_color(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HidBackend {
    #[default]
    Hidraw,
    Rusb,
}

impl std::fmt::Display for HidBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hidraw => write!(f, "hidraw"),
            Self::Rusb => write!(f, "rusb"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_fps")]
    pub default_fps: f32,
    #[serde(default, skip_serializing)]
    pub hid_driver: Option<String>,
    #[serde(default)]
    pub hid_backend: HidBackend,
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
    /// Per-ENE 6K77 device configuration keyed by device serial.
    #[serde(default)]
    pub ene6k77: HashMap<String, Ene6k77DeviceConfig>,
    #[serde(default)]
    pub wireless_groups: HashMap<String, WirelessGroupConfig>,
    #[serde(default)]
    pub thermal_alert: ThermalAlertSettings,
    /// Wireless RGB drift re-sync: re-applies saved RGB when a wireless
    /// device's firmware resets its lighting. Wireless devices only.
    #[serde(default = "default_true")]
    pub rgb_drift_detection_enabled: bool,
    #[serde(default = "default_drift_interval_ms")]
    pub rgb_drift_detection_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WirelessGroupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb_rgb_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb_pwm_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_palette: Option<Vec<[u8; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_fan_lcd_enable: Option<u8>,
}

fn default_true() -> bool {
    true
}

fn default_drift_interval_ms() -> u64 {
    1000
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_fps: default_fps(),
            hid_driver: None,
            hid_backend: HidBackend::default(),
            lcds: Vec::new(),
            fan_curves: Vec::new(),
            fans: None,
            rgb: None,
            aio: HashMap::new(),
            ene6k77: HashMap::new(),
            wireless_groups: HashMap::new(),
            thermal_alert: ThermalAlertSettings::default(),
            rgb_drift_detection_enabled: default_true(),
            rgb_drift_detection_interval_ms: default_drift_interval_ms(),
        }
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

        // Collapse entries whose serials differ only by the "hid:" prefix,
        // keeping the prefixed form when both exist. Canonicalizing bare
        // serials happens in the daemon, where the device list is available.
        let prefixed_keys: HashSet<String> = cfg
            .lcds
            .iter()
            .filter_map(|d| d.serial.as_deref())
            .filter_map(|s| s.strip_prefix("hid:"))
            .map(str::to_string)
            .collect();
        let mut seen_serials: HashSet<String> = HashSet::new();
        cfg.lcds.retain(|device| {
            let Some(serial) = device.serial.clone() else {
                return true;
            };
            let is_prefixed = serial.starts_with("hid:");
            let bare = serial.strip_prefix("hid:").unwrap_or(&serial).to_string();
            let duplicate = !seen_serials.insert(bare.clone());
            let shadowed = !is_prefixed && prefixed_keys.contains(&bare);
            if duplicate || shadowed {
                let kept = if !is_prefixed && prefixed_keys.contains(&bare) {
                    " (kept hid:-prefixed form)"
                } else {
                    ""
                };
                warnings.push(format!(
                    "Removed duplicate LCD entry for serial '{bare}'{kept}"
                ));
                if shadowed {
                    // Free the key so the prefixed entry can claim it.
                    seen_serials.remove(&bare);
                }
                return false;
            }
            true
        });

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
        let aio = AioConfig {
            pump_target_rpm: group.speeds[3].clone(),
            fan_speeds: group.speeds,
            ..Default::default()
        };
        self.aio.insert(aio_device_id.to_string(), aio);
        true
    }
}

pub type ConfigKey = String;

pub fn config_identity(cfg: &LcdConfig) -> ConfigKey {
    to_string(cfg).unwrap_or_else(|_| cfg.device_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_from(json: &str) -> (AppConfig, Vec<String>) {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lianli-shared-config-test-{}-{n}.json",
            std::process::id()
        ));
        std::fs::write(&path, json).unwrap();
        let result = AppConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        result.unwrap()
    }

    fn lcd(serial: &str) -> String {
        format!(r#"{{"serial": "{serial}", "type": "color", "rgb": [0, 0, 0]}}"#)
    }

    #[test]
    fn bare_serial_left_untouched() {
        let (cfg, _) = load_from(&format!(r#"{{"lcds": [{}]}}"#, lcd("00000055FA92")));
        assert_eq!(cfg.lcds.len(), 1);
        assert_eq!(cfg.lcds[0].serial.as_deref(), Some("00000055FA92"));
    }

    #[test]
    fn hid_prefixed_duplicate_wins_over_bare() {
        let (cfg, warnings) = load_from(&format!(
            r#"{{"lcds": [{}, {}]}}"#,
            lcd("00000055FA92"),
            lcd("hid:00000055FA92")
        ));
        assert_eq!(cfg.lcds.len(), 1);
        assert_eq!(cfg.lcds[0].serial.as_deref(), Some("hid:00000055FA92"));
        assert!(warnings.iter().any(|w| w.contains("duplicate")));
    }

    #[test]
    fn prefixed_first_keeps_bare_duplicate_dropped() {
        let (cfg, _) = load_from(&format!(
            r#"{{"lcds": [{}, {}]}}"#,
            lcd("hid:00000055FA92"),
            lcd("00000055FA92")
        ));
        assert_eq!(cfg.lcds.len(), 1);
        assert_eq!(cfg.lcds[0].serial.as_deref(), Some("hid:00000055FA92"));
    }

    #[test]
    fn prefixed_prefixed_duplicate_second_dropped() {
        let (cfg, warnings) = load_from(&format!(
            r#"{{"lcds": [{}, {}]}}"#,
            lcd("hid:00000055FA92"),
            lcd("hid:00000055FA92")
        ));
        assert_eq!(cfg.lcds.len(), 1);
        assert_eq!(cfg.lcds[0].serial.as_deref(), Some("hid:00000055FA92"));
        assert!(warnings.iter().any(|w| w.contains("duplicate")));
    }

    #[test]
    fn distinct_bare_serials_both_kept() {
        let (cfg, warnings) = load_from(&format!(
            r#"{{"lcds": [{}, {}]}}"#,
            lcd("00000055FA92"),
            lcd("000000AA42BB")
        ));
        assert_eq!(cfg.lcds.len(), 2);
        assert_eq!(cfg.lcds[0].serial.as_deref(), Some("00000055FA92"));
        assert_eq!(cfg.lcds[1].serial.as_deref(), Some("000000AA42BB"));
        assert!(!warnings.iter().any(|w| w.contains("duplicate")));
    }

    #[test]
    fn wireless_and_path_serials_untouched() {
        let (cfg, _) = load_from(&format!(
            r#"{{"lcds": [{}, {}]}}"#,
            lcd("wireless:AA:BB:CC:DD:EE:FF"),
            lcd("3-8.2.1")
        ));
        assert_eq!(cfg.lcds.len(), 2);
        assert_eq!(
            cfg.lcds[0].serial.as_deref(),
            Some("wireless:AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(cfg.lcds[1].serial.as_deref(), Some("3-8.2.1"));
    }
}
