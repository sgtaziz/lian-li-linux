use crate::sensors::TempSource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanCurve {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_source: Option<TempSource>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub temp_command: String,
    pub curve: Vec<(f32, f32)>,
}

impl FanCurve {
    pub fn effective_source(&self) -> TempSource {
        if let Some(ref source) = self.temp_source {
            source.clone()
        } else if !self.temp_command.is_empty() {
            TempSource::Command {
                cmd: self.temp_command.clone(),
            }
        } else {
            TempSource::Command {
                cmd: "cat /sys/class/thermal/thermal_zone0/temp | awk '{print $1/1000}'".into(),
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FanSpeed {
    Constant(u8),
    Curve(String),
}

/// Reserved curve name used to represent motherboard RPM sync mode.
pub const MB_SYNC_KEY: &str = "__mb_sync__";

impl FanSpeed {
    /// True if this speed represents motherboard RPM sync mode.
    pub fn is_mb_sync(&self) -> bool {
        matches!(self, FanSpeed::Curve(name) if name == MB_SYNC_KEY)
    }
}

/// A fan speed group targeting a specific device.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanGroup {
    /// Device identifier (e.g. "wireless:AA:BB:CC:DD:EE:FF" or "usb:1:5" or a serial).
    /// When absent, groups are matched by index order to discovered devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// PWM per fan slot (up to 4).
    pub speeds: [FanSpeed; 4],
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanConfig {
    #[serde(deserialize_with = "deserialize_fan_groups")]
    pub speeds: Vec<FanGroup>,
    #[serde(default = "default_update_interval")]
    pub update_interval_ms: u64,
}

impl Default for FanConfig {
    fn default() -> Self {
        Self {
            speeds: vec![],
            update_interval_ms: default_update_interval(),
        }
    }
}

fn default_update_interval() -> u64 {
    100
}

/// Custom deserializer: accepts either the new `Vec<FanGroup>` format
/// or the legacy `Vec<[FanSpeed; 4]>` (array of arrays) for backward compat.
fn deserialize_fan_groups<'de, D>(deserializer: D) -> Result<Vec<FanGroup>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct FanGroupsVisitor;

    impl<'de> Visitor<'de> for FanGroupsVisitor {
        type Value = Vec<FanGroup>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an array of fan groups or an array of fan speed arrays")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut result = Vec::new();

            while let Some(val) = seq.next_element::<serde_json::Value>()? {
                if val.is_object() {
                    // New format: { device_id: "...", speeds: [...] }
                    let group: FanGroup = serde_json::from_value(val)
                        .map_err(|e| de::Error::custom(format!("Invalid fan group: {e}")))?;
                    result.push(group);
                } else if val.is_array() {
                    // Legacy format: [speed, speed, speed, speed]
                    let speeds: [FanSpeed; 4] = serde_json::from_value(val)
                        .map_err(|e| de::Error::custom(format!("Invalid fan speed array: {e}")))?;
                    result.push(FanGroup {
                        device_id: None,
                        speeds,
                    });
                } else {
                    return Err(de::Error::custom(
                        "Expected a fan group object or speed array",
                    ));
                }
            }

            Ok(result)
        }
    }

    deserializer.deserialize_seq(FanGroupsVisitor)
}
