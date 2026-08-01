use crate::aio::AioConfig;
use crate::config::Ene6k77DeviceConfig;
use crate::fan::{FanConfig, FanGroup};
use crate::rgb::RgbDeviceConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub name: String,
    pub device_id: String,
    #[serde(default)]
    pub device_family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<RgbDeviceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aio: Option<AioConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_group: Option<FanGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ene6k77: Option<Ene6k77DeviceConfig>,
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

        let fan_group = config
            .fans
            .as_ref()
            .and_then(|f| {
                f.speeds
                    .iter()
                    .find(|g| g.device_id.as_deref() == Some(device_id))
                    .cloned()
            })
            .or_else(|| {
                if device_family == "WirelessAio"
                    || device_family == "Galahad2Trinity"
                    || device_family == "HydroShiftLcd"
                    || device_family == "Galahad2Lcd"
                    || device_family == "HydroShift2Lcd"
                    || device_family == "HydroShift2OledCurveLed"
                {
                    None
                } else {
                    None
                }
            });

        let ene6k77 = config.ene6k77.iter().find_map(|(k, v)| {
            if device_id.contains(k.as_str()) {
                Some(v.clone())
            } else {
                None
            }
        });

        Self {
            name: name.to_string(),
            device_id: device_id.to_string(),
            device_family: device_family.to_string(),
            rgb,
            aio,
            fan_group,
            ene6k77,
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
            let key = self
                .device_id
                .rsplit(':')
                .next()
                .unwrap_or(&self.device_id)
                .to_string();
            config.ene6k77.insert(key, ene.clone());
        }
    }
}
