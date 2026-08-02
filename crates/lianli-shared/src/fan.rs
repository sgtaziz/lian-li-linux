use crate::sensors::SensorSource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanCurve {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_source: Option<SensorSource>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub temp_command: String,
    pub curve: Vec<(f32, f32)>,
}

impl FanCurve {
    pub fn effective_source(&self) -> SensorSource {
        if let Some(ref source) = self.temp_source {
            if matches!(source, SensorSource::Command { .. }) {
                SensorSource::Command {
                    cmd: self.temp_command.clone(),
                }
            } else {
                source.clone()
            }
        } else if !self.temp_command.is_empty() {
            SensorSource::Command {
                cmd: self.temp_command.clone(),
            }
        } else {
            SensorSource::Command {
                cmd: "cat /sys/class/thermal/thermal_zone0/temp | awk '{print $1/1000}'".into(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FanSpeed {
    Constant(u8),
    Curve(String),
}

/// Reserved curve name used to represent motherboard RPM sync mode.
pub const MB_SYNC_KEY: &str = "__mb_sync__";

/// Prefix for MB sync with a specific hwmon PWM source.
pub const MB_SYNC_PREFIX: &str = "__mb_sync__:";

/// Piecewise-linear interpolation across a fan/AIO curve.
///
/// - Empty curve → `50.0` (safe mid-PWM fallback).
/// - Single point → that point's speed.
/// - Temp below/above the curve's range → clamped to the first/last point.
/// - Otherwise → linear interpolation between the bracketing points.
///
/// The input curve does not need to be sorted; this function sorts a copy.
pub fn interpolate_curve(curve: &[(f32, f32)], temp: f32) -> f32 {
    if curve.is_empty() {
        return 50.0;
    }
    if curve.len() == 1 {
        return curve[0].1;
    }

    let mut sorted = curve.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    if temp <= sorted[0].0 {
        return sorted[0].1;
    }
    let last = sorted.len() - 1;
    if temp >= sorted[last].0 {
        return sorted[last].1;
    }

    for i in 0..last {
        let (t1, s1) = sorted[i];
        let (t2, s2) = sorted[i + 1];
        if temp >= t1 && temp <= t2 {
            let ratio = (temp - t1) / (t2 - t1);
            return s1 + ratio * (s2 - s1);
        }
    }
    50.0
}

/// Map a 0..=255 PWM duty to a 0..=100 percentage.
///
/// Used by AIO drivers whose firmware reports PWM in 0..=255 but expects
/// fan-speed arguments as a percentage.
#[inline]
pub fn duty_to_percent(duty: u8) -> u8 {
    ((duty as u32 * 100) / 255) as u8
}

impl FanSpeed {
    pub fn is_mb_sync(&self) -> bool {
        match self {
            FanSpeed::Curve(name) => name == MB_SYNC_KEY || name.starts_with(MB_SYNC_PREFIX),
            _ => false,
        }
    }

    pub fn mb_sync_source(&self) -> Option<&str> {
        match self {
            FanSpeed::Curve(name) if name.starts_with(MB_SYNC_PREFIX) => {
                Some(&name[MB_SYNC_PREFIX.len()..])
            }
            _ => None,
        }
    }
}

/// A fan speed group targeting a specific device.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanGroup {
    /// Device identifier (e.g. "wireless:AA:BB:CC:DD:EE:FF" or "usb:1:5" or a serial).
    /// When absent, groups are matched by index order to discovered devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// PWM per fan slot (up to 4). Hubs with fewer than 4 physically
    /// connected fans (e.g. a 3-fan Uni Hub) report shorter arrays from the
    /// frontend; unpopulated trailing slots are padded with `Constant(0)`.
    #[serde(deserialize_with = "deserialize_padded_speeds")]
    pub speeds: [FanSpeed; 4],
}

/// Pads a fan-speed list of length 0..=4 out to exactly 4 slots, filling any
/// missing trailing slots with `FanSpeed::Constant(0)` (unused/off). Hubs
/// with fewer than 4 physically connected fans (e.g. a 3-fan Uni Hub) report
/// shorter arrays from the frontend than the fixed 4-slot wire format.
fn pad_speeds(mut speeds: Vec<FanSpeed>) -> Result<[FanSpeed; 4], String> {
    if speeds.len() > 4 {
        return Err(format!(
            "invalid length {}, expected at most 4 fan speeds",
            speeds.len()
        ));
    }
    while speeds.len() < 4 {
        speeds.push(FanSpeed::Constant(0));
    }
    Ok(speeds
        .try_into()
        .expect("padded to exactly 4 elements above"))
}

/// Deserializes a fan-speed array of length 0..=4, padding any missing
/// trailing slots with `FanSpeed::Constant(0)` (unused/off).
fn deserialize_padded_speeds<'de, D>(deserializer: D) -> Result<[FanSpeed; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let speeds: Vec<FanSpeed> = Vec::deserialize(deserializer)?;
    pad_speeds(speeds).map_err(D::Error::custom)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanConfig {
    #[serde(deserialize_with = "deserialize_fan_groups")]
    pub speeds: Vec<FanGroup>,
    #[serde(default = "default_update_interval")]
    pub update_interval_ms: u64,
    #[serde(default = "default_hysteresis_temp")]
    pub hysteresis_temp: f32,
    #[serde(default = "default_hysteresis_pwm")]
    pub hysteresis_pwm: u8,
}

impl Default for FanConfig {
    fn default() -> Self {
        Self {
            speeds: vec![],
            update_interval_ms: default_update_interval(),
            hysteresis_temp: default_hysteresis_temp(),
            hysteresis_pwm: default_hysteresis_pwm(),
        }
    }
}

fn default_update_interval() -> u64 {
    1000
}

fn default_hysteresis_temp() -> f32 {
    1.0
}

fn default_hysteresis_pwm() -> u8 {
    5
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
                    // Legacy format: [speed, speed, speed, speed] (or fewer,
                    // for hubs with less than 4 physically connected fans).
                    let speeds_vec: Vec<FanSpeed> = serde_json::from_value(val)
                        .map_err(|e| de::Error::custom(format!("Invalid fan speed array: {e}")))?;
                    let speeds = pad_speeds(speeds_vec).map_err(de::Error::custom)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_curve_empty_returns_default() {
        assert_eq!(interpolate_curve(&[], 50.0), 50.0);
    }

    #[test]
    fn interpolate_curve_single_point_returns_it() {
        assert_eq!(interpolate_curve(&[(20.0, 42.0)], 50.0), 42.0);
    }

    #[test]
    fn interpolate_curve_clamps_below_range() {
        let curve = [(30.0, 25.0), (60.0, 100.0)];
        assert_eq!(interpolate_curve(&curve, 10.0), 25.0);
    }

    #[test]
    fn interpolate_curve_clamps_above_range() {
        let curve = [(30.0, 25.0), (60.0, 100.0)];
        assert_eq!(interpolate_curve(&curve, 90.0), 100.0);
    }

    #[test]
    fn interpolate_curve_interpolates_inside() {
        let curve = [(30.0, 25.0), (60.0, 100.0)];
        // halfway: 25 + 0.5 * (100 - 25) = 62.5
        assert!((interpolate_curve(&curve, 45.0) - 62.5).abs() < 1e-6);
    }

    #[test]
    fn interpolate_curve_works_unsorted() {
        let curve = [(60.0, 100.0), (30.0, 25.0)];
        assert!((interpolate_curve(&curve, 45.0) - 62.5).abs() < 1e-6);
    }

    #[test]
    fn interpolate_curve_handles_multi_segment() {
        let curve = [(20.0, 0.0), (40.0, 50.0), (80.0, 100.0)];
        assert!((interpolate_curve(&curve, 30.0) - 25.0).abs() < 1e-6);
        assert!((interpolate_curve(&curve, 60.0) - 75.0).abs() < 1e-6);
    }

    #[test]
    fn duty_to_percent_endpoints() {
        assert_eq!(duty_to_percent(0), 0);
        assert_eq!(duty_to_percent(255), 100);
    }

    #[test]
    fn duty_to_percent_midpoint() {
        // 127 / 255 ≈ 49.8 → floored to 49
        assert_eq!(duty_to_percent(127), 49);
    }

    #[test]
    fn duty_to_percent_quarter() {
        // 64 / 255 ≈ 25.1 → floored to 25
        assert_eq!(duty_to_percent(64), 25);
    }

    #[test]
    fn fan_group_object_format_pads_short_speeds_array() {
        // A 3-fan hub (e.g. a Uni Hub with only 3 connected fans) reports a
        // 3-element speeds array, which must be padded rather than rejected.
        let json = r#"[{"device_id": "wireless:AA:BB:CC:DD:EE:01", "speeds": [30, 40, 50]}]"#;
        let groups: Vec<FanGroup> = serde_json::from_str(&format!(
            r#"{{"speeds": {json}}}"#
        ))
        .map(|c: FanConfig| c.speeds)
        .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].speeds,
            [
                FanSpeed::Constant(30),
                FanSpeed::Constant(40),
                FanSpeed::Constant(50),
                FanSpeed::Constant(0),
            ]
        );
    }

    #[test]
    fn fan_group_legacy_array_format_pads_short_speeds_array() {
        let json = r#"{"speeds": [[30, 40]]}"#;
        let groups: Vec<FanGroup> = serde_json::from_str(json)
            .map(|c: FanConfig| c.speeds)
            .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].speeds,
            [
                FanSpeed::Constant(30),
                FanSpeed::Constant(40),
                FanSpeed::Constant(0),
                FanSpeed::Constant(0),
            ]
        );
    }

    #[test]
    fn fan_group_speeds_array_too_long_is_rejected() {
        let json = r#"{"speeds": [{"speeds": [10, 20, 30, 40, 50]}]}"#;
        let result: Result<FanConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
