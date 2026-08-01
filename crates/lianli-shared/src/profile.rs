use serde::{Deserialize, Serialize};

use crate::aio::AioConfig;
use crate::config::{Ene6k77DeviceConfig, LcdConfig, ThermalAlertSettings};
use crate::fan::{FanConfig, FanGroup};
use crate::rgb::RgbDeviceConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub name: String,
    pub device_id: String,
    #[serde(default)]
    pub device_family: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<RgbDeviceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lcds: Vec<LcdConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aio: Option<AioConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_group: Option<FanGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ene6k77: Option<Ene6k77DeviceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_alert: Option<ThermalAlertSettings>,
}

fn default_schema_version() -> u32 {
    1
}

impl DeviceProfile {
    pub fn capture_from_config(
        config: &crate::config::AppConfig,
        name: &str,
        device_id: &str,
        device_family: &str,
    ) -> Self {
        let rgb = config
            .rgb
            .as_ref()
            .and_then(|r| r.devices.iter().find(|d| d.device_id == device_id).cloned());

        let aio = config.aio.get(device_id).cloned();

        let fan_group = config.fans.as_ref().and_then(|f| {
            f.speeds
                .iter()
                .find(|g| g.device_id.as_deref() == Some(device_id))
                .cloned()
        });

        let ene6k77 = ene6k77_lookup(config, device_id);

        let lcds: Vec<LcdConfig> = config
            .lcds
            .iter()
            .filter(|lcd| {
                if let Some(serial) = &lcd.serial {
                    device_id.contains(serial.as_str())
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        let thermal_alert = Some(config.thermal_alert.clone());

        Self {
            name: name.to_string(),
            device_id: device_id.to_string(),
            device_family: device_family.to_string(),
            schema_version: 1,
            rgb,
            lcds,
            aio,
            fan_group,
            ene6k77,
            thermal_alert,
        }
    }

    pub fn apply_to_config(&self, config: &mut crate::config::AppConfig) {
        if let Some(rgb) = &self.rgb {
            let rgb_cfg = config.rgb.get_or_insert_with(Default::default);
            if let Some(existing) = rgb_cfg
                .devices
                .iter_mut()
                .find(|d| d.device_id == self.device_id)
            {
                *existing = rgb.clone();
            } else {
                rgb_cfg.devices.push(rgb.clone());
            }
        }

        if let Some(aio) = &self.aio {
            config.aio.insert(self.device_id.clone(), aio.clone());
        }

        if let Some(fan_group) = &self.fan_group {
            let fans = config.fans.get_or_insert_with(|| FanConfig {
                speeds: Vec::new(),
                update_interval_ms: 1000,
                hysteresis_temp: 3.0,
                hysteresis_pwm: 5,
            });
            if let Some(existing) = fans
                .speeds
                .iter_mut()
                .find(|g| g.device_id.as_deref() == Some(&self.device_id))
            {
                *existing = fan_group.clone();
            } else {
                let mut g = fan_group.clone();
                g.device_id = Some(self.device_id.clone());
                fans.speeds.push(g);
            }
        }

        if let Some(ene) = &self.ene6k77 {
            if let Some(key) = ene6k77_key_for(&self.device_id) {
                config.ene6k77.insert(key, ene.clone());
            }
        }

        if !self.lcds.is_empty() {
            for lcd in &self.lcds {
                if let Some(idx) = config.lcds.iter().position(|existing| {
                    existing.serial == lcd.serial || existing.index == lcd.index
                }) {
                    config.lcds[idx] = lcd.clone();
                } else {
                    config.lcds.push(lcd.clone());
                }
            }
        }

        if let Some(ta) = &self.thermal_alert {
            config.thermal_alert = ta.clone();
        }
    }
}

fn ene6k77_lookup(
    config: &crate::config::AppConfig,
    device_id: &str,
) -> Option<Ene6k77DeviceConfig> {
    for (key, val) in &config.ene6k77 {
        if device_id.contains(key.as_str()) {
            return Some(val.clone());
        }
    }
    None
}

fn ene6k77_key_for(device_id: &str) -> Option<String> {
    device_id
        .split(':')
        .last()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
