use crate::aio::AioConfig;
use crate::device_id::DeviceFamily;
use crate::fan::{FanConfig, FanCurve};
use crate::media::{
    DoublegaugeDescriptor, MediaFraming, MediaType, SensorDescriptor, SensorSourceConfig,
};
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
    #[serde(default, skip_serializing_if = "MediaFraming::is_default")]
    pub framing: MediaFraming,
}

impl LcdConfig {
    pub fn resolve_paths(&mut self, base: &Path) {
        crate::media_dependencies::map_lcd_paths(self, |path| {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        });
    }

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
        self.validate_settings()?;
        if self.media_type == MediaType::Sensor {
            if let Some(sensor) = &self.sensor {
                sensor.validate()?;
            }
        }
        if matches!(
            self.media_type,
            MediaType::Image | MediaType::Video | MediaType::Gif
        ) {
            if let Some(path) = &self.path {
                if !path.exists() {
                    bail!(
                        "LCD[{}] media path '{}' does not exist",
                        self.device_id(),
                        path.display()
                    );
                }
            }
        }
        Ok(())
    }

    pub fn validate_settings(&self) -> Result<()> {
        if self.index.is_none() && self.serial.is_none() {
            bail!("device config requires either 'index' or 'serial' field");
        }

        let device_id = self.device_id();

        match self.media_type {
            MediaType::Image | MediaType::Video | MediaType::Gif => {
                self.path
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("LCD[{device_id}] requires a media path"))?;
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
                descriptor.validate_settings()?;
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
            if !fps.is_finite() || fps <= 0.0 {
                bail!("LCD[{device_id}] fps must be finite and positive");
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
    #[serde(default = "default_true")]
    pub turn_off_lcds_on_shutdown: bool,
    #[serde(default = "default_fps")]
    pub default_fps: f32,
    #[serde(default)]
    pub hardware_video: bool,
    #[serde(default, skip_serializing)]
    pub hid_driver: Option<String>,
    #[serde(default)]
    pub hid_backend: HidBackend,
    #[serde(
        default,
        alias = "devices",
        deserialize_with = "crate::serde_limits::lcds"
    )]
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
    /// ENE controller IDs; legacy serial keys migrate when ownership is unambiguous.
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
            turn_off_lcds_on_shutdown: default_true(),
            default_fps: default_fps(),
            hardware_video: false,
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
        Self::from_reader(reader, path)
    }

    /// Applies startup migrations and resolves media paths against the destination file.
    pub fn from_reader(reader: impl std::io::Read, path: &Path) -> Result<(Self, Vec<String>)> {
        use std::io::Read;
        const MAX_BYTES: u64 = 16 * 1024 * 1024;
        let mut bytes = Vec::new();
        reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Configuration exceeds 16 MiB"
        );
        let mut cfg: AppConfig = serde_json::from_slice(&bytes)
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

            device.resolve_paths(&base_dir);

            if let Some(sensor) = &mut device.sensor {
                // Preserve the legacy sensor cadence in the shared LCD field.
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
    #[test]
    fn hardware_video_defaults_off_and_round_trips_explicit_choices() {
        let legacy: super::AppConfig = serde_json::from_str("{}").unwrap();
        assert!(!legacy.hardware_video);
        assert!(!super::AppConfig::default().hardware_video);
        for enabled in [true, false] {
            let config = super::AppConfig {
                hardware_video: enabled,
                ..Default::default()
            };
            let encoded = serde_json::to_value(&config).unwrap();
            assert_eq!(encoded["hardware_video"], enabled);
            let decoded: super::AppConfig = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded.hardware_video, enabled);
        }
    }

    use super::*;

    #[test]
    fn lcd_shutdown_defaults_on_and_preserves_explicit_false() {
        assert!(AppConfig::default().turn_off_lcds_on_shutdown);
        let legacy: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(legacy.turn_off_lcds_on_shutdown);
        for enabled in [false, true] {
            let config: AppConfig = serde_json::from_value(serde_json::json!({
                "turn_off_lcds_on_shutdown": enabled
            }))
            .unwrap();
            assert_eq!(config.turn_off_lcds_on_shutdown, enabled);
            let saved = serde_json::to_value(&config).unwrap();
            assert_eq!(saved["turn_off_lcds_on_shutdown"], enabled);
        }
    }

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
    fn in_memory_config_uses_destination_paths_and_enforces_the_size_limit() {
        let json = br#"{"lcds":[{"index":0,"type":"video","path":"clip.mp4","orientation":100}]}"#;
        let (config, _) =
            AppConfig::from_reader(&json[..], Path::new("/state/config.json")).unwrap();
        assert_eq!(
            config.lcds[0].path.as_deref(),
            Some(Path::new("/state/clip.mp4"))
        );
        assert_eq!(config.lcds[0].orientation, 90.0);
        assert!(
            AppConfig::from_reader(std::io::repeat(b' '), Path::new("config.json"))
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
    }

    #[test]
    fn settings_validation_does_not_require_assets_to_exist_but_requires_valid_values() {
        let sensor: LcdConfig = serde_json::from_str(
            r#"{"index":0,"type":"sensor","sensor":{"label":"Load","unit":"%","source":{"type":"constant","value":50},"font_path":"/missing-lianli-validation-font.ttf"}}"#,
        ).unwrap();
        assert!(sensor.validate_settings().is_ok());
        assert!(sensor.validate().is_err());
        let mut config: LcdConfig = serde_json::from_str(
            r#"{"index":0,"type":"video","path":"/missing-lianli-validation-asset.mp4"}"#,
        )
        .unwrap();
        assert!(config.validate_settings().is_ok());
        config.path = None;
        assert!(config.validate_settings().is_err());
        config.media_type = MediaType::Color;
        config.rgb = Some([0, 0, 0]);
        for fps in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            config.fps = Some(fps);
            assert!(config.validate_settings().is_err());
        }
        config.fps = Some(30.0);
        assert!(config.validate_settings().is_ok());
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
