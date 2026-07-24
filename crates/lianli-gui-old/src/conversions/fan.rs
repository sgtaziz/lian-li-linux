use super::device::family_display_name;
use lianli_shared::fan::{FanConfig, FanCurve, FanSpeed};
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::sensors::Unit;
use slint::{ModelRc, SharedString, VecModel};

const TEMP_MIN: f32 = 20.0;
const TEMP_MAX: f32 = 100.0;

/// Build line segments between consecutive sorted points.
pub fn build_curve_segments(sorted: &[(f32, f32)]) -> Vec<crate::CurveSegment> {
    sorted
        .windows(2)
        .map(|w| crate::CurveSegment {
            from_temp: w[0].0,
            from_speed: w[0].1,
            to_temp: w[1].0,
            to_speed: w[1].1,
        })
        .collect()
}

/// Build clamp segments extending horizontally from the first/last point to axis edges.
pub fn build_clamp_segments(sorted: &[(f32, f32)]) -> Vec<crate::CurveSegment> {
    let mut segs = Vec::new();
    if sorted.is_empty() {
        return segs;
    }
    let first = sorted[0];
    if first.0 > TEMP_MIN {
        segs.push(crate::CurveSegment {
            from_temp: TEMP_MIN,
            from_speed: first.1,
            to_temp: first.0,
            to_speed: first.1,
        });
    }
    let last = sorted[sorted.len() - 1];
    if last.0 < TEMP_MAX {
        segs.push(crate::CurveSegment {
            from_temp: last.0,
            from_speed: last.1,
            to_temp: TEMP_MAX,
            to_speed: last.1,
        });
    }
    segs
}

pub fn fan_curve_to_slint(
    curve: &FanCurve,
    sensors: &[lianli_shared::sensors::SensorInfo],
) -> crate::FanCurveData {
    let points: Vec<crate::CurvePoint> = curve
        .curve
        .iter()
        .map(|&(temp, speed)| crate::CurvePoint { temp, speed })
        .collect();

    let mut sorted: Vec<(f32, f32)> = curve.curve.clone();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let sensor = curve.temp_source.as_ref();

    let mut sensor_index = 0;
    let user_cmd = curve.temp_command.to_string();
    if let Some(sd) = sensor {
        // Find sd in sensors: Return its index
        if let Some(idx) = sensors
            .iter()
            .filter(|s| s.unit == Unit::C)
            .position(|si| si.source == *sd)
        {
            sensor_index = idx;
        } else {
            sensor_index = sensors.iter().filter(|s| s.unit == Unit::C).count();
        }
    }

    crate::FanCurveData {
        name: SharedString::from(&curve.name),
        temp_source_index: sensor_index as i32,
        temp_command: SharedString::from(user_cmd),
        points: ModelRc::new(VecModel::from(points)),
        curve_segments: ModelRc::new(VecModel::from(build_curve_segments(&sorted))),
        clamp_segments: ModelRc::new(VecModel::from(build_clamp_segments(&sorted))),
    }
}

pub fn fan_curves_to_model(
    curves: &[FanCurve],
    sensors: &[lianli_shared::sensors::SensorInfo],
) -> ModelRc<crate::FanCurveData> {
    let items: Vec<_> = curves
        .iter()
        .map(|c| fan_curve_to_slint(c, sensors))
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn sensor_options_model(
    sensors: &[lianli_shared::sensors::SensorInfo],
    only_temp_sensors: bool,
) -> ModelRc<SharedString> {
    let mut items: Vec<SharedString> = sensors
        .iter()
        .filter(|s| !only_temp_sensors || s.unit == Unit::C)
        .enumerate()
        .map(|(i, s)| {
            let display_name = format!("{}. {}", i + 1, s.get_display_name());
            SharedString::from(display_name)
        })
        .collect();
    let display_name = format!("{}. {}", items.len() + 1, "Custom command");
    items.push(SharedString::from(display_name));
    ModelRc::new(VecModel::from(items))
}

pub fn curve_names_to_model(curves: &[FanCurve]) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = curves.iter().map(|c| SharedString::from(&c.name)).collect();
    ModelRc::new(VecModel::from(items))
}

pub fn font_options_model() -> ModelRc<SharedString> {
    let mut items: Vec<SharedString> =
        vec![SharedString::from(lianli_shared::fonts::DEFAULT_FONT_LABEL)];
    items.extend(
        lianli_shared::fonts::cached_system_fonts()
            .iter()
            .map(|f| SharedString::from(f.family.as_str())),
    );
    ModelRc::new(VecModel::from(items))
}

/// Build the speed options dropdown list: ["Off", curve1, curve2, ..., "Constant PWM", "MB Sync"]
pub fn speed_options_model(curves: &[FanCurve], _has_mb_sync: bool) -> ModelRc<SharedString> {
    let mut items = vec![SharedString::from("Off")];
    for c in curves {
        items.push(SharedString::from(&c.name));
    }
    items.push(SharedString::from("Constant PWM"));
    items.push(SharedString::from("MB Sync"));
    ModelRc::new(VecModel::from(items))
}

pub(super) fn fan_speed_to_slot(
    s: &FanSpeed,
    pwm_headers: &[lianli_shared::sensors::PwmHeader],
) -> crate::FanSpeedSlot {
    if s.is_mb_sync() {
        let source_id = s.mb_sync_source().unwrap_or("");
        let label = pwm_headers
            .iter()
            .find(|h| h.id == source_id)
            .map(|h| {
                let pct = lianli_shared::sensors::read_pwm_header(&h.id)
                    .map(|v| (v as f32 / 255.0 * 100.0).round() as u8)
                    .unwrap_or(0);
                format!("{} ({}%)", h.label, pct)
            })
            .unwrap_or_default();
        return crate::FanSpeedSlot {
            dropdown_value: SharedString::from("MB Sync"),
            pwm_percent: 0,
            display_mode: SharedString::from("mb_sync"),
            pwm_header: SharedString::from(source_id),
            pwm_header_label: SharedString::from(&label),
        };
    }
    match s {
        FanSpeed::Constant(0) => crate::FanSpeedSlot {
            dropdown_value: SharedString::from("Off"),
            pwm_percent: 0,
            display_mode: SharedString::from("off"),
            pwm_header: SharedString::default(),
            pwm_header_label: SharedString::default(),
        },
        FanSpeed::Constant(pwm) => crate::FanSpeedSlot {
            dropdown_value: SharedString::from("Constant PWM"),
            pwm_percent: ((*pwm as f32 / 255.0) * 100.0).round() as i32,
            display_mode: SharedString::from("constant"),
            pwm_header: SharedString::default(),
            pwm_header_label: SharedString::default(),
        },
        FanSpeed::Curve(name) => crate::FanSpeedSlot {
            dropdown_value: SharedString::from(name.as_str()),
            pwm_percent: 0,
            display_mode: SharedString::from("curve"),
            pwm_header: SharedString::default(),
            pwm_header_label: SharedString::default(),
        },
    }
}

const DEFAULT_SPEEDS: [FanSpeed; 4] = [
    FanSpeed::Constant(0),
    FanSpeed::Constant(0),
    FanSpeed::Constant(0),
    FanSpeed::Constant(0),
];

pub fn fan_groups_to_model(
    fan_config: &FanConfig,
    devices: &[DeviceInfo],
    pwm_headers: &[lianli_shared::sensors::PwmHeader],
) -> ModelRc<crate::FanGroupData> {
    let fan_devices: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| (d.has_fan && d.fan_count.unwrap_or(0) > 0) || d.has_pump_control)
        .filter(|d| d.pump_rpm_range.is_none())
        .collect();

    let items: Vec<crate::FanGroupData> = fan_devices
        .iter()
        .map(|dev| {
            let group = fan_config
                .speeds
                .iter()
                .find(|g| g.device_id.as_deref() == Some(&dev.device_id));
            let speeds = group.map(|g| &g.speeds[..]).unwrap_or(&DEFAULT_SPEEDS);

            let device_name = if dev.name.is_empty() {
                family_display_name(dev.family).to_string()
            } else {
                dev.name.clone()
            };

            let slots: Vec<crate::FanSpeedSlot> = speeds
                .iter()
                .map(|s| fan_speed_to_slot(s, pwm_headers))
                .collect();

            let pump_slot = if dev.has_pump_control {
                fan_speed_to_slot(speeds.get(3).unwrap_or(&FanSpeed::Constant(0)), pwm_headers)
            } else {
                fan_speed_to_slot(&FanSpeed::Constant(0), pwm_headers)
            };

            crate::FanGroupData {
                device_id: SharedString::from(&dev.device_id),
                device_name: SharedString::from(&device_name),
                fan_count: dev.fan_count.unwrap_or(4) as i32,
                per_fan_control: dev.per_fan_control.unwrap_or(false),
                mb_sync_support: dev.mb_sync_support,
                is_wireless: dev.device_id.starts_with("wireless:"),
                has_pump_control: dev.has_pump_control,
                pump_slot,
                slots: ModelRc::new(VecModel::from(slots)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}
