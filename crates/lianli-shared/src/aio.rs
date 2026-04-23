use crate::fan::FanSpeed;
use crate::media::SensorSourceConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const AIO_PIC_MAX_BYTES: usize = 20_480;
pub const AIO_PIC_DIMENSION: u32 = 480;

fn default_brightness() -> u8 {
    80
}

fn default_loop_interval() -> u8 {
    3
}

fn default_pump_speed() -> FanSpeed {
    FanSpeed::Constant(128)
}

fn default_fan_speeds() -> [FanSpeed; 4] {
    [
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
        FanSpeed::Constant(128),
    ]
}

fn rgba_white() -> [u8; 4] {
    [255, 255, 255, 255]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AioConfig {
    #[serde(default = "default_pump_speed")]
    pub pump_target_rpm: FanSpeed,
    #[serde(default = "default_fan_speeds")]
    pub fan_speeds: [FanSpeed; 4],
    #[serde(default)]
    pub theme_index: u8,
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    #[serde(default)]
    pub rotation: u8,
    #[serde(default = "default_loop_interval")]
    pub loop_interval: u8,
    #[serde(default)]
    pub cpu_temp_source: Option<SensorSourceConfig>,
    #[serde(default = "default_cpu_load_source")]
    pub cpu_load_source: Option<SensorSourceConfig>,
    #[serde(default)]
    pub gpu_temp_source: Option<SensorSourceConfig>,
    #[serde(default)]
    pub gpu_load_source: Option<SensorSourceConfig>,
    #[serde(default = "rgba_white")]
    pub str_color: [u8; 4],
    #[serde(default = "rgba_white")]
    pub val_color: [u8; 4],
    #[serde(default = "rgba_white")]
    pub unit_color: [u8; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_image_path: Option<PathBuf>,
}

fn default_cpu_load_source() -> Option<SensorSourceConfig> {
    Some(SensorSourceConfig::CpuUsage)
}

impl Default for AioConfig {
    fn default() -> Self {
        Self {
            pump_target_rpm: default_pump_speed(),
            fan_speeds: default_fan_speeds(),
            theme_index: 0,
            brightness: default_brightness(),
            rotation: 0,
            loop_interval: default_loop_interval(),
            cpu_temp_source: None,
            cpu_load_source: default_cpu_load_source(),
            gpu_temp_source: None,
            gpu_load_source: None,
            str_color: rgba_white(),
            val_color: rgba_white(),
            unit_color: rgba_white(),
            custom_image_path: None,
        }
    }
}

impl AioConfig {
    pub fn defaults_for_host() -> Self {
        use crate::sensors::{enumerate_sensors, pick_source_for_category, SensorCategory};
        let sensors = enumerate_sensors();
        let mut cfg = Self::default();
        cfg.cpu_temp_source = pick_source_for_category(SensorCategory::CpuTemp, &sensors);
        cfg.cpu_load_source =
            pick_source_for_category(SensorCategory::CpuUsage, &sensors).or(cfg.cpu_load_source);
        cfg.gpu_temp_source = pick_source_for_category(SensorCategory::GpuTemp, &sensors);
        cfg.gpu_load_source = pick_source_for_category(SensorCategory::GpuUsage, &sensors);
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_json() {
        let cfg = AioConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AioConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.brightness, cfg.brightness);
        assert_eq!(back.rotation, cfg.rotation);
        assert_eq!(back.loop_interval, cfg.loop_interval);
        assert_eq!(back.theme_index, cfg.theme_index);
        assert_eq!(back.str_color, cfg.str_color);
        assert_eq!(back.val_color, cfg.val_color);
        assert_eq!(back.unit_color, cfg.unit_color);
    }

    #[test]
    fn sparse_json_fills_defaults() {
        let cfg: AioConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.brightness, 80);
        assert_eq!(cfg.loop_interval, 3);
        assert_eq!(cfg.str_color, [255, 255, 255, 255]);
        assert!(matches!(cfg.cpu_load_source, Some(SensorSourceConfig::CpuUsage)));
    }
}
