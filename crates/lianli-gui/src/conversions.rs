//! Conversions between lianli-shared Rust types and Slint-generated structs.

use lianli_shared::config::{AppConfig, LcdConfig};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::fan::{FanConfig, FanCurve, FanSpeed};
use lianli_shared::ipc::{DeviceInfo, TelemetrySnapshot};
use lianli_shared::media::MediaType;
use lianli_shared::rgb::{RgbDeviceCapabilities, RgbMode, RgbScope};
use lianli_shared::sensors::Unit;
use slint::{ModelRc, SharedString, VecModel};

fn family_display_name(f: DeviceFamily) -> &'static str {
    match f {
        DeviceFamily::Ene6k77 => "UNI FAN SL/AL",
        DeviceFamily::TlFan => "UNI FAN TL",
        DeviceFamily::TlLcd => "UNI FAN TL LCD",
        DeviceFamily::Galahad2Trinity => "Galahad II Trinity",
        DeviceFamily::HydroShiftLcd => "HydroShift LCD",
        DeviceFamily::Galahad2Lcd => "Galahad II LCD",
        DeviceFamily::WirelessTx => "Wireless TX Dongle",
        DeviceFamily::WirelessRx => "Wireless RX Dongle",
        DeviceFamily::Slv3Lcd => "UNI FAN SL Wireless LCD",
        DeviceFamily::Slv3Led => "UNI FAN SL Wireless",
        DeviceFamily::Tlv2Lcd => "UNI FAN TL Wireless LCD",
        DeviceFamily::Tlv2Led => "UNI FAN TL Wireless",
        DeviceFamily::SlInf => "UNI FAN SL-INF Wireless",
        DeviceFamily::Clv1 => "UNI FAN CL Wireless",
        DeviceFamily::HydroShift2Lcd => "HydroShift II LCD Circle",
        DeviceFamily::Lancool207 => "Lancool 207 Digital",
        DeviceFamily::UniversalScreen => "Universal Screen 8.8\"",
        DeviceFamily::HydroShift2LcdDesktop => "HydroShift II LCD (Desktop Mode)",
        DeviceFamily::Lancool207Desktop => "Lancool 207 Digital (Desktop Mode)",
        DeviceFamily::UniversalScreenDesktop => "Universal Screen 8.8\" (Desktop Mode)",
        DeviceFamily::WirelessAio => "HydroShift Wireless AIO",
        DeviceFamily::WirelessStrimer => "Strimer Plus Wireless",
        DeviceFamily::WirelessLc217 => "Lancool 217 Wireless",
        DeviceFamily::WirelessLed88 => "Universal Screen 8.8\" Wireless",
        DeviceFamily::WirelessV150 => "Lancool V150 Wireless",
        DeviceFamily::StrimerPlus => "Strimer Plus",
        DeviceFamily::UniversalScreenLighting => "Universal Screen 8.8\" LED Ring",
    }
}

pub fn device_to_slint(device: &DeviceInfo, telemetry: &TelemetrySnapshot) -> super::DeviceData {
    let fan_rpms = telemetry
        .fan_rpms
        .get(&device.device_id)
        .map(|rpms| {
            rpms.iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let coolant_temp = telemetry
        .coolant_temps
        .get(&device.device_id)
        .map(|t| format!("{t:.1}\u{00B0}C"))
        .unwrap_or_default();

    let resolution = match (device.screen_width, device.screen_height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => String::new(),
    };

    let family_name = family_display_name(device.family);

    super::DeviceData {
        device_id: SharedString::from(&device.device_id),
        family_name: SharedString::from(family_name),
        name: SharedString::from(&device.name),
        serial: SharedString::from(device.serial.as_deref().unwrap_or("")),
        has_lcd: device.has_lcd,
        has_fan: device.has_fan,
        has_pump: device.has_pump,
        has_rgb: device.has_rgb,
        fan_rpms: SharedString::from(&fan_rpms),
        coolant_temp: SharedString::from(&coolant_temp),
        resolution: SharedString::from(&resolution),
        in_desktop_mode: device.family.is_desktop_mode(),
        in_lcd_mode: device.family.supports_display_mode_switch()
            && !device.family.is_desktop_mode(),
        is_unbound_wireless: device.is_unbound_wireless,
    }
}

pub fn devices_to_model(
    devices: &[DeviceInfo],
    telemetry: &TelemetrySnapshot,
) -> ModelRc<super::DeviceData> {
    let items: Vec<super::DeviceData> = devices
        .iter()
        .filter(|d| {
            !matches!(
                d.family,
                lianli_shared::device_id::DeviceFamily::WirelessTx
                    | lianli_shared::device_id::DeviceFamily::WirelessRx
            )
        })
        .map(|d| device_to_slint(d, telemetry))
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn media_type_to_string(mt: &MediaType) -> &'static str {
    match mt {
        MediaType::Image => "Image",
        MediaType::Video => "Video",
        MediaType::Gif => "GIF",
        MediaType::Color => "Solid Color",
        MediaType::Sensor => "Sensor Gauge",
        MediaType::Custom | MediaType::Doublegauge | MediaType::Cooler => "Custom",
    }
}

pub fn lcd_to_slint(
    lcd: &LcdConfig,
    devices: &[DeviceInfo],
    sensors: &[lianli_shared::sensors::SensorInfo],
) -> super::LcdEntryData {
    let sensor = lcd.sensor.as_ref();

    let mut sg_sensor_index = 0;
    let mut cmd = "".to_string();
    if let Some(sd) = sensor {
        let ts: lianli_shared::sensors::SensorSource = sd.source.to_sensor_source();
        if let Some(idx) = sensors.iter().position(|si| si.source == ts) {
            sg_sensor_index = idx;
        } else {
            sg_sensor_index = sensors.len();
            cmd = match ts {
                lianli_shared::sensors::SensorSource::Command { cmd } => cmd,
                _ => String::new(),
            };
        }
    };

    let text_color = sensor.map(|s| s.text_color).unwrap_or([255, 255, 255]);
    let bg_color = sensor.map(|s| s.background_color).unwrap_or([0, 0, 0]);
    let gauge_bg = sensor
        .map(|s| s.gauge_background_color)
        .unwrap_or([40, 40, 40]);

    let gauge_ranges: Vec<super::GaugeRangeData> = sensor
        .map(|s| {
            s.gauge_ranges
                .iter()
                .map(|r| super::GaugeRangeData {
                    max_value: r.max.unwrap_or(100.0) as i32,
                    r: r.color[0] as i32,
                    g: r.color[1] as i32,
                    b: r.color[2] as i32,
                })
                .collect()
        })
        .unwrap_or_default();

    let [r, g, b] = lcd.rgb.unwrap_or([0, 0, 0]);
    let serial_str = lcd.serial.as_deref().unwrap_or("");
    let device_label = lcd_serial_to_label(serial_str, devices);

    super::LcdEntryData {
        serial: SharedString::from(serial_str),
        device_label: SharedString::from(&device_label),
        media_type: SharedString::from(media_type_to_string(&lcd.media_type)),
        path: SharedString::from(
            lcd.path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
        fps: lcd.fps.map(|f| f as i32).unwrap_or(30),
        orientation: lcd.orientation as i32,
        rgb_r: r as i32,
        rgb_g: g as i32,
        rgb_b: b as i32,
        sensor_label: SharedString::from(sensor.map(|s| s.label.as_str()).unwrap_or("")),
        sensor_unit: SharedString::from(sensor.map(|s| s.unit.as_str()).unwrap_or("")),
        sg_sensor_index: sg_sensor_index as i32,
        sensor_command: SharedString::from(&cmd),
        sensor_font_path: SharedString::from(
            sensor
                .and_then(|s| s.font_path.as_ref())
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
        sensor_font_name: SharedString::from(lianli_shared::fonts::font_label_for_path(
            sensor.and_then(|s| s.font_path.as_deref()),
        )),
        sensor_decimal_places: sensor.map(|s| s.decimal_places as i32).unwrap_or(0),
        update_interval_ms: lcd.update_interval_ms.unwrap_or(1000) as i32,
        sensor_value_font_size: sensor.map(|s| s.value_font_size as i32).unwrap_or(120),
        sensor_unit_font_size: sensor.map(|s| s.unit_font_size as i32).unwrap_or(40),
        sensor_label_font_size: sensor.map(|s| s.label_font_size as i32).unwrap_or(30),
        sensor_start_angle: sensor.map(|s| s.gauge_start_angle as i32).unwrap_or(135),
        sensor_sweep_angle: sensor.map(|s| s.gauge_sweep_angle as i32).unwrap_or(270),
        sensor_outer_radius: sensor.map(|s| s.gauge_outer_radius as i32).unwrap_or(200),
        sensor_thickness: sensor.map(|s| s.gauge_thickness as i32).unwrap_or(30),
        sensor_corner_radius: sensor.map(|s| s.bar_corner_radius as i32).unwrap_or(5),
        sensor_value_offset: sensor.map(|s| s.value_offset).unwrap_or(0),
        sensor_unit_offset: sensor.map(|s| s.unit_offset).unwrap_or(0),
        sensor_label_offset: sensor.map(|s| s.label_offset).unwrap_or(0),
        sensor_text_color_r: text_color[0] as i32,
        sensor_text_color_g: text_color[1] as i32,
        sensor_text_color_b: text_color[2] as i32,
        sensor_bg_color_r: bg_color[0] as i32,
        sensor_bg_color_g: bg_color[1] as i32,
        sensor_bg_color_b: bg_color[2] as i32,
        sensor_gauge_bg_r: gauge_bg[0] as i32,
        sensor_gauge_bg_g: gauge_bg[1] as i32,
        sensor_gauge_bg_b: gauge_bg[2] as i32,
        sensor_gauge_ranges: ModelRc::new(VecModel::from(gauge_ranges)),

        template_id: SharedString::from(lcd.template_id.as_deref().unwrap_or("")),
        template_name: SharedString::default(),
        template_preview: slint::Image::default(),
    }
}

pub fn lcd_entries_to_model(
    lcds: &[LcdConfig],
    devices: &[DeviceInfo],
    sensors: &[lianli_shared::sensors::SensorInfo],
    templates: &[lianli_shared::template::LcdTemplate],
) -> ModelRc<super::LcdEntryData> {
    let items: Vec<_> = lcds
        .iter()
        .map(|l| {
            let mut entry = lcd_to_slint(l, devices, sensors);
            if let Some(tid) = &l.template_id {
                if let Some(tpl) = templates.iter().find(|t| &t.id == tid) {
                    entry.template_name = SharedString::from(tpl.name.as_str());
                    if let Some(path) =
                        lianli_shared::template_catalog::template_preview_path(&tpl.id)
                    {
                        if path.exists() {
                            if let Ok(img) = slint::Image::load_from_path(&path) {
                                entry.template_preview = img;
                            }
                        }
                    }
                }
            }
            entry
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Pretty-label list of templates for the LCD page Custom dropdown.
/// Order mirrors `templates` — Rust callback side resolves label → id by
/// linear scan of the same slice.
pub fn template_labels_model(
    templates: &[lianli_shared::template::LcdTemplate],
) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = templates
        .iter()
        .map(|t| SharedString::from(t.name.as_str()))
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Resolve a pretty label back to the template id. Used by the
/// `update_lcd_field("template_label", ...)` handler.
pub fn template_id_for_label(
    label: &str,
    templates: &[lianli_shared::template::LcdTemplate],
) -> Option<String> {
    templates
        .iter()
        .find(|t| t.name == label)
        .map(|t| t.id.clone())
}

/// Format a device option label for LCD device selector: "FriendlyName (serial)"
pub fn lcd_device_label(device: &DeviceInfo) -> String {
    let name = if device.name.is_empty() {
        family_display_name(device.family).to_string()
    } else {
        device.name.clone()
    };
    let serial = device.serial.as_deref().unwrap_or(&device.device_id);
    format!("{name} ({serial})")
}

/// Build device option strings for LCD device selector.
pub fn lcd_device_options(devices: &[DeviceInfo]) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = devices
        .iter()
        .filter(|d| d.has_lcd)
        .map(|d| SharedString::from(lcd_device_label(d)))
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Find the serial for a given LCD device label, or return the label as-is.
pub fn lcd_label_to_serial(label: &str, devices: &[DeviceInfo]) -> String {
    devices
        .iter()
        .filter(|d| d.has_lcd)
        .find(|d| lcd_device_label(d) == label)
        .map(|d| d.serial.clone().unwrap_or_else(|| d.device_id.clone()))
        .unwrap_or_else(|| label.to_string())
}

/// Find the display label for a given serial.
pub fn lcd_serial_to_label(serial: &str, devices: &[DeviceInfo]) -> String {
    devices
        .iter()
        .filter(|d| d.has_lcd)
        .find(|d| d.serial.as_deref() == Some(serial) || d.device_id == serial)
        .map(|d| lcd_device_label(d))
        .unwrap_or_else(|| serial.to_string())
}

const TEMP_MIN: f32 = 20.0;
const TEMP_MAX: f32 = 100.0;

/// Build line segments between consecutive sorted points.
pub fn build_curve_segments(sorted: &[(f32, f32)]) -> Vec<super::CurveSegment> {
    sorted
        .windows(2)
        .map(|w| super::CurveSegment {
            from_temp: w[0].0,
            from_speed: w[0].1,
            to_temp: w[1].0,
            to_speed: w[1].1,
        })
        .collect()
}

/// Build clamp segments extending horizontally from the first/last point to axis edges.
pub fn build_clamp_segments(sorted: &[(f32, f32)]) -> Vec<super::CurveSegment> {
    let mut segs = Vec::new();
    if sorted.is_empty() {
        return segs;
    }
    let first = sorted[0];
    if first.0 > TEMP_MIN {
        segs.push(super::CurveSegment {
            from_temp: TEMP_MIN,
            from_speed: first.1,
            to_temp: first.0,
            to_speed: first.1,
        });
    }
    let last = sorted[sorted.len() - 1];
    if last.0 < TEMP_MAX {
        segs.push(super::CurveSegment {
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
) -> super::FanCurveData {
    let points: Vec<super::CurvePoint> = curve
        .curve
        .iter()
        .map(|&(temp, speed)| super::CurvePoint { temp, speed })
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

    super::FanCurveData {
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
) -> ModelRc<super::FanCurveData> {
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

fn fan_speed_to_slot(
    s: &FanSpeed,
    pwm_headers: &[lianli_shared::sensors::PwmHeader],
) -> super::FanSpeedSlot {
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
        return super::FanSpeedSlot {
            dropdown_value: SharedString::from("MB Sync"),
            pwm_percent: 0,
            display_mode: SharedString::from("mb_sync"),
            pwm_header: SharedString::from(source_id),
            pwm_header_label: SharedString::from(&label),
        };
    }
    match s {
        FanSpeed::Constant(0) => super::FanSpeedSlot {
            dropdown_value: SharedString::from("Off"),
            pwm_percent: 0,
            display_mode: SharedString::from("off"),
            pwm_header: SharedString::default(),
            pwm_header_label: SharedString::default(),
        },
        FanSpeed::Constant(pwm) => super::FanSpeedSlot {
            dropdown_value: SharedString::from("Constant PWM"),
            pwm_percent: ((*pwm as f32 / 255.0) * 100.0).round() as i32,
            display_mode: SharedString::from("constant"),
            pwm_header: SharedString::default(),
            pwm_header_label: SharedString::default(),
        },
        FanSpeed::Curve(name) => super::FanSpeedSlot {
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
) -> ModelRc<super::FanGroupData> {
    let fan_devices: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| (d.has_fan && d.fan_count.unwrap_or(0) > 0) || d.has_pump_control)
        .collect();

    let items: Vec<super::FanGroupData> = fan_devices
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

            let slots: Vec<super::FanSpeedSlot> = speeds
                .iter()
                .map(|s| fan_speed_to_slot(s, pwm_headers))
                .collect();

            let pump_slot = if dev.has_pump_control {
                fan_speed_to_slot(speeds.get(3).unwrap_or(&FanSpeed::Constant(0)), pwm_headers)
            } else {
                fan_speed_to_slot(&FanSpeed::Constant(0), pwm_headers)
            };

            super::FanGroupData {
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

fn rgb_mode_to_string(mode: &RgbMode) -> String {
    format!("{mode:?}")
}

pub fn rgb_devices_to_model(
    capabilities: &[RgbDeviceCapabilities],
    config: &AppConfig,
    presets: &[lianli_shared::rgb::RgbPreset],
) -> ModelRc<super::RgbDeviceData> {
    let rgb_config = config.rgb.as_ref();
    let device_configs = rgb_config.map(|r| &r.devices);

    let items: Vec<super::RgbDeviceData> = capabilities
        .iter()
        .map(|cap| {
            let dev_cfg =
                device_configs.and_then(|devs| devs.iter().find(|d| d.device_id == cap.device_id));

            let mb_rgb_sync = dev_cfg.map(|d| d.mb_rgb_sync).unwrap_or(false);

            // Determine if device has group zones (scoped: Top/Bottom or Inner/Outer)
            let has_group_zones = cap.supported_scopes.iter().any(|scopes| {
                scopes.iter().any(|s| {
                    matches!(
                        s,
                        RgbScope::Top | RgbScope::Bottom | RgbScope::Inner | RgbScope::Outer
                    )
                })
            });

            // Check zone 0 config to determine synced state
            let z0_cfg = dev_cfg.and_then(|d| d.zones.iter().find(|z| z.zone_index == 0));
            let synced = if has_group_zones {
                if let Some(zcfg) = z0_cfg {
                    let is_per_fan = matches!(
                        zcfg.effect.mode,
                        RgbMode::Off | RgbMode::Static | RgbMode::Direct
                    ) && matches!(zcfg.effect.scope, RgbScope::All);
                    !is_per_fan
                } else {
                    false
                }
            } else {
                false
            };

            let zones: Vec<super::RgbZoneData> = cap
                .zones
                .iter()
                .enumerate()
                .map(|(zidx, zone_info)| {
                    let zone_cfg =
                        dev_cfg.and_then(|d| d.zones.iter().find(|z| z.zone_index == zidx as u8));

                    let (mode, colors, speed, brightness, direction, scope, swap_lr, swap_tb) =
                        if let Some(zcfg) = zone_cfg {
                            let e = &zcfg.effect;
                            let colors: Vec<super::RgbColorData> = e
                                .colors
                                .iter()
                                .map(|c| super::RgbColorData {
                                    r: c[0] as i32,
                                    g: c[1] as i32,
                                    b: c[2] as i32,
                                })
                                .collect();
                            (
                                rgb_mode_to_string(&e.mode),
                                colors,
                                e.speed as i32,
                                e.brightness as i32,
                                format!("{:?}", e.direction),
                                format!("{:?}", e.scope),
                                zcfg.swap_lr,
                                zcfg.swap_tb,
                            )
                        } else {
                            (
                                "Off".to_string(),
                                vec![super::RgbColorData { r: 255, g: 0, b: 0 }],
                                2,
                                3,
                                "Clockwise".to_string(),
                                "All".to_string(),
                                false,
                                false,
                            )
                        };

                    let led_colors: Vec<super::RgbColorData> = if mode == "Direct" {
                        let base = colors.first().cloned().unwrap_or(super::RgbColorData {
                            r: 0,
                            g: 0,
                            b: 0,
                        });
                        vec![base; zone_info.led_count as usize]
                    } else {
                        Vec::new()
                    };

                    super::RgbZoneData {
                        zone_index: zidx as i32,
                        zone_name: SharedString::from(&zone_info.name),
                        led_count: zone_info.led_count as i32,
                        mode: SharedString::from(&mode),
                        colors: ModelRc::new(VecModel::from(colors)),
                        led_colors: ModelRc::new(VecModel::from(led_colors)),
                        speed,
                        brightness,
                        direction: SharedString::from(&direction),
                        scope: SharedString::from(&scope),
                        swap_lr,
                        swap_tb,
                        is_synced_zone: synced && zidx != 0,
                    }
                })
                .collect();

            let supported_modes: Vec<SharedString> = cap
                .supported_modes
                .iter()
                .map(|m| SharedString::from(rgb_mode_to_string(m)))
                .collect();

            // Flatten all scopes across zones into a unique set
            let mut all_scopes: Vec<String> = cap
                .supported_scopes
                .iter()
                .flat_map(|s| s.iter().map(|sc| format!("{sc:?}")))
                .collect();
            all_scopes.sort();
            all_scopes.dedup();
            let supported_scopes: Vec<SharedString> = all_scopes
                .iter()
                .map(|s| SharedString::from(s.as_str()))
                .collect();

            let preset_names: Vec<SharedString> = presets
                .iter()
                .filter(|p| p.device_id == cap.device_id)
                .map(|p| SharedString::from(&p.name))
                .collect();

            let active_preset = dev_cfg
                .and_then(|d| d.active_preset.as_deref())
                .unwrap_or("");

            super::RgbDeviceData {
                device_id: SharedString::from(&cap.device_id),
                device_name: SharedString::from(&cap.device_name),
                total_leds: cap.total_led_count as i32,
                mb_rgb_sync: mb_rgb_sync,
                supports_mb_sync: cap.supports_mb_rgb_sync,
                supports_direction: cap.supports_direction,
                has_group_zones,
                synced,
                is_wireless: cap.device_id.starts_with("wireless:"),
                active_preset: SharedString::from(active_preset),
                supported_modes: ModelRc::new(VecModel::from(supported_modes)),
                supported_scopes: ModelRc::new(VecModel::from(supported_scopes)),
                preset_names: ModelRc::new(VecModel::from(preset_names)),
                zones: ModelRc::new(VecModel::from(zones)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}
