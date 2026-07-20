use super::device::family_display_name;
use lianli_shared::config::LcdConfig;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::media::MediaType;
use slint::{ModelRc, SharedString, VecModel};

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
) -> crate::LcdEntryData {
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

    let gauge_ranges: Vec<crate::GaugeRangeData> = sensor
        .map(|s| {
            s.gauge_ranges
                .iter()
                .map(|r| crate::GaugeRangeData {
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
    let device_supports_h264 = devices
        .iter()
        .find(|d| d.serial.as_deref() == Some(serial_str))
        .and_then(|d| lianli_shared::screen::screen_info_for(d.family))
        .map(|s| s.h264)
        .unwrap_or(false);
    let supports_c_command = devices
        .iter()
        .find(|d| d.serial.as_deref() == Some(serial_str))
        .map(|d| d.supports_c_command)
        .unwrap_or(false);

    crate::LcdEntryData {
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
        smooth_edges: lcd.smooth_edges(),
        custom_h264: lcd.custom_h264(),
        device_supports_h264,
        supports_c_command,
        aio_512_frame: lcd.aio_512_frame(),
    }
}

pub fn lcd_entries_to_model(
    lcds: &[LcdConfig],
    devices: &[DeviceInfo],
    sensors: &[lianli_shared::sensors::SensorInfo],
    templates: &[lianli_shared::template::LcdTemplate],
) -> ModelRc<crate::LcdEntryData> {
    let items: Vec<_> = lcds
        .iter()
        .map(|l| {
            let mut entry = lcd_to_slint(l, devices, sensors);
            if let Some(tid) = &l.template_id {
                if let Some(tpl) = templates.iter().find(|t| &t.id == tid) {
                    entry.template_name = SharedString::from(tpl.name.as_str());
                    if let Some(path) =
                        lianli_shared::template::catalog::template_preview_path(&tpl.id)
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
    if let Some((port, fan)) = device.port_index {
        return format!("{name} (port {port}, fan {fan})");
    }
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
        .map(lcd_device_label)
        .unwrap_or_else(|| serial.to_string())
}
