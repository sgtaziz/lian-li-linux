use super::template::{delete_current_template, duplicate_current_template};
use crate::conversions;
use crate::editor;
use crate::template_browser;
use crate::{MainWindow, Shared};
use lianli_shared::media::SensorSourceConfig;
use slint::ComponentHandle;

pub(crate) fn wire_lcd_callbacks(
    window: &MainWindow,
    shared: &Shared,
    editor: &editor::EditorHandle,
    browser: &template_browser::BrowserHandle,
) {
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_add_lcd(move || {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    c.lcds.push(lianli_shared::config::LcdConfig {
                        index: None,
                        serial: None,
                        media_type: lianli_shared::media::MediaType::Image,
                        path: None,
                        fps: Some(30.0),
                        update_interval_ms: None,
                        rgb: None,
                        orientation: 0.0,
                        sensor_source_1: SensorSourceConfig::CpuUsage,
                        sensor_source_2: SensorSourceConfig::MemUsage,
                        sensor: None,
                        doublegauge: None,
                        template_id: None,
                        smooth_edges: None,
                        custom_h264: None,
                        aio_512_frame: None,
                    });
                }
            }
            crate::refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_remove_lcd(move |idx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if idx < c.lcds.len() {
                        c.lcds.remove(idx);
                    }
                }
            }
            crate::refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_update_lcd_field(move |idx, field, val| {
            let field_str = field.to_string();
            // Only rebuild UI for dropdown/button fields that affect layout.
            // Text fields update in-place in the LineEdit — rebuilding would steal focus.
            let needs_refresh = matches!(
                field_str.as_str(),
                "device"
                    | "media_type"
                    | "orientation"
                    | "sensor_source"
                    | "template_label"
                    | "template_id"
            ) || field_str == "gauge_range_add"
                || field_str == "gauge_range_remove";
            {
                let mut state = shared.lock().unwrap();
                let devices = state.devices.clone();
                let templates_snapshot = state.lcd_templates.clone();
                let resolved_sensor_source: Option<lianli_shared::media::SensorSourceConfig> = {
                    let val_str = val.to_string();
                    let sensor_idx: usize = val_str
                        .split('.')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let is_sensor_picker = field_str == "sensor_source";
                    if is_sensor_picker && !val_str.ends_with("Custom command") && sensor_idx > 0 {
                        state.available_sensors.get(sensor_idx - 1).map(|sensor| {
                            match &sensor.source {
                                lianli_shared::sensors::SensorSource::Hwmon {
                                    name,
                                    label,
                                    device_path,
                                } => lianli_shared::media::SensorSourceConfig::Hwmon {
                                    name: name.clone(),
                                    label: label.clone(),
                                    device_path: device_path.clone(),
                                },
                                lianli_shared::sensors::SensorSource::NvidiaGpu {
                                    gpu_index,
                                    metric,
                                } => lianli_shared::media::SensorSourceConfig::NvidiaGpu {
                                    gpu_index: *gpu_index,
                                    metric: *metric,
                                },
                                lianli_shared::sensors::SensorSource::AmdGpuUsage {
                                    card_index,
                                } => lianli_shared::media::SensorSourceConfig::AmdGpuUsage {
                                    card_index: *card_index,
                                },
                                lianli_shared::sensors::SensorSource::Command { cmd } => {
                                    lianli_shared::media::SensorSourceConfig::Command {
                                        cmd: cmd.clone(),
                                    }
                                }
                                lianli_shared::sensors::SensorSource::WirelessCoolant {
                                    device_id,
                                } => lianli_shared::media::SensorSourceConfig::WirelessCoolant {
                                    device_id: device_id.clone(),
                                },
                                lianli_shared::sensors::SensorSource::CpuUsage => {
                                    lianli_shared::media::SensorSourceConfig::CpuUsage
                                }
                                lianli_shared::sensors::SensorSource::MemUsage => {
                                    lianli_shared::media::SensorSourceConfig::MemUsage
                                }
                                lianli_shared::sensors::SensorSource::MemUsed => {
                                    lianli_shared::media::SensorSourceConfig::MemUsed
                                }
                                lianli_shared::sensors::SensorSource::MemFree => {
                                    lianli_shared::media::SensorSourceConfig::MemFree
                                }
                                lianli_shared::sensors::SensorSource::NetworkRate {
                                    iface,
                                    direction,
                                } => match direction {
                                    lianli_shared::sensors::NetDirection::Rx => {
                                        lianli_shared::media::SensorSourceConfig::NetworkRx {
                                            iface: iface.clone(),
                                        }
                                    }
                                    lianli_shared::sensors::NetDirection::Tx => {
                                        lianli_shared::media::SensorSourceConfig::NetworkTx {
                                            iface: iface.clone(),
                                        }
                                    }
                                },
                                lianli_shared::sensors::SensorSource::DiskRate {
                                    device,
                                    direction,
                                } => match direction {
                                    lianli_shared::sensors::DiskDirection::Read => {
                                        lianli_shared::media::SensorSourceConfig::DiskRead {
                                            device: device.clone(),
                                        }
                                    }
                                    lianli_shared::sensors::DiskDirection::Write => {
                                        lianli_shared::media::SensorSourceConfig::DiskWrite {
                                            device: device.clone(),
                                        }
                                    }
                                },
                            }
                        })
                    } else {
                        None
                    }
                };
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if let Some(lcd) = c.lcds.get_mut(idx) {
                        let val = val.to_string();
                        match field_str.as_str() {
                            "device" => {
                                // Resolve label back to serial
                                let serial = conversions::lcd_label_to_serial(&val, &devices);
                                lcd.serial = Some(serial);
                            }
                            "media_type" => {
                                lcd.media_type = match val.as_str() {
                                    "Image" => lianli_shared::media::MediaType::Image,
                                    "Video" => lianli_shared::media::MediaType::Video,
                                    "GIF" => lianli_shared::media::MediaType::Gif,
                                    "Solid Color" => {
                                        lcd.rgb.get_or_insert([0, 0, 0]);
                                        lianli_shared::media::MediaType::Color
                                    }
                                    "Sensor Gauge" => {
                                        lcd.sensor.get_or_insert_with(default_sensor);
                                        lcd.path = None;
                                        lianli_shared::media::MediaType::Sensor
                                    }
                                    "Custom" => {
                                        lcd.path = None;
                                        lianli_shared::media::MediaType::Custom
                                    }
                                    _ => lcd.media_type,
                                };
                            }
                            "path" => lcd.path = Some(std::path::PathBuf::from(val)),
                            "orientation" => lcd.orientation = val.parse().unwrap_or(0.0),
                            "template_label" => {
                                // Resolve label → id via the snapshot taken
                                // before the mutable borrow of `state.config`.
                                if let Some(id) =
                                    conversions::template_id_for_label(&val, &templates_snapshot)
                                {
                                    lcd.template_id = Some(id);
                                }
                            }
                            "template_id" => {
                                lcd.template_id = Some(val);
                            }
                            "sensor_label" => {
                                lcd.sensor.get_or_insert_with(default_sensor).label = val;
                            }
                            "sensor_unit" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit = val;
                            }
                            "sensor_source" => {
                                let sensor_cfg = lcd.sensor.get_or_insert_with(default_sensor);
                                sensor_cfg.source = resolved_sensor_source.clone().unwrap_or(
                                    lianli_shared::media::SensorSourceConfig::Command {
                                        cmd: String::new(),
                                    },
                                );
                            }
                            "sensor_command" => {
                                lcd.sensor.get_or_insert_with(default_sensor).source =
                                    lianli_shared::media::SensorSourceConfig::Command { cmd: val };
                            }
                            "sensor_font_path" => {
                                lcd.sensor.get_or_insert_with(default_sensor).font_path =
                                    Some(std::path::PathBuf::from(val));
                            }
                            "sensor_font_name" => {
                                lcd.sensor.get_or_insert_with(default_sensor).font_path =
                                    lianli_shared::fonts::font_path_for_label(&val);
                            }
                            "fps" => lcd.fps = Some(val.parse::<f32>().unwrap_or(30.0)),
                            "smooth_edges" => lcd.smooth_edges = Some(val == "true"),
                            "custom_h264" => lcd.custom_h264 = Some(val == "true"),
                            "aio_512_frame" => lcd.aio_512_frame = Some(val == "true"),
                            "rgb_r" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[0] = val.parse().unwrap_or(0)
                            }
                            "rgb_g" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[1] = val.parse().unwrap_or(0)
                            }
                            "rgb_b" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[2] = val.parse().unwrap_or(0)
                            }
                            "sensor_decimal_places" => {
                                lcd.sensor.get_or_insert_with(default_sensor).decimal_places =
                                    val.parse().unwrap_or(0);
                            }
                            "update_interval_ms" => {
                                lcd.update_interval_ms =
                                    Some(val.parse().unwrap_or(1000).clamp(100, 10_000));
                            }
                            "sensor_value_font_size" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .value_font_size = val.parse().unwrap_or(120.0);
                            }
                            "sensor_unit_font_size" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit_font_size =
                                    val.parse().unwrap_or(40.0);
                            }
                            "sensor_label_font_size" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .label_font_size = val.parse().unwrap_or(30.0);
                            }
                            "sensor_start_angle" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_start_angle = val.parse().unwrap_or(135.0);
                            }
                            "sensor_sweep_angle" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_sweep_angle = val.parse().unwrap_or(270.0);
                            }
                            "sensor_outer_radius" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_outer_radius = val.parse().unwrap_or(200.0);
                            }
                            "sensor_thickness" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_thickness = val.parse().unwrap_or(30.0);
                            }
                            "sensor_corner_radius" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .bar_corner_radius = val.parse().unwrap_or(5.0);
                            }
                            "sensor_value_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).value_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_unit_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_label_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).label_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_text_color_r" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[0] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_text_color_g" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[1] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_text_color_b" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[2] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_bg_color_r" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[0] = val.parse().unwrap_or(0)
                            }
                            "sensor_bg_color_g" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[1] = val.parse().unwrap_or(0)
                            }
                            "sensor_bg_color_b" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[2] = val.parse().unwrap_or(0)
                            }
                            "sensor_gauge_bg_r" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[0] = val.parse().unwrap_or(40)
                            }
                            "sensor_gauge_bg_g" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[1] = val.parse().unwrap_or(40)
                            }
                            "sensor_gauge_bg_b" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[2] = val.parse().unwrap_or(40)
                            }
                            "gauge_range_add" => {
                                let s = lcd.sensor.get_or_insert_with(default_sensor);
                                s.gauge_ranges.push(lianli_shared::media::SensorRange {
                                    max: Some(100.0),
                                    color: [0, 200, 0],
                                    alpha: 255,
                                });
                            }

                            f if f.starts_with("gauge_range_remove") => {
                                if let Ok(ridx) = val.parse::<usize>() {
                                    let s = lcd.sensor.get_or_insert_with(default_sensor);
                                    if ridx < s.gauge_ranges.len() {
                                        s.gauge_ranges.remove(ridx);
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_max_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_max_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<f32>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.max = Some(v);
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_r_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_r_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[0] = v;
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_g_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_g_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[1] = v;
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_b_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_b_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[2] = v;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if needs_refresh {
                crate::refresh_lcd_ui(&weak, &shared);
            } else if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_pick_lcd_file(move |idx| {
            let shared2 = shared.clone();
            let weak2 = weak.clone();
            let idx = idx as usize;
            std::thread::spawn(move || {
                let is_sensor = {
                    let state = shared2.lock().unwrap();
                    state
                        .config
                        .as_ref()
                        .and_then(|c| c.lcds.get(idx))
                        .map(|lcd| lcd.media_type == lianli_shared::media::MediaType::Sensor)
                        .unwrap_or(false)
                };
                let mut dialog = rfd::FileDialog::new();
                dialog = if is_sensor {
                    dialog.add_filter("Images", &["jpg", "jpeg", "png", "bmp"])
                } else {
                    dialog.add_filter(
                        "Media",
                        &[
                            "jpg", "jpeg", "png", "apng", "bmp", "gif", "mp4", "avi", "mkv",
                            "webm", "mov",
                        ],
                    )
                };
                let file = dialog.pick_file();
                if let Some(path) = file {
                    {
                        let mut state = shared2.lock().unwrap();
                        if let Some(ref mut c) = state.config {
                            if let Some(lcd) = c.lcds.get_mut(idx) {
                                lcd.path = Some(path);
                            }
                        }
                    }
                    crate::refresh_lcd_ui(&weak2, &shared2);
                }
            });
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        let editor_window = editor.window.clone_strong();
        let editor_state = editor.state.clone();
        window.on_lcd_create_template(move |idx| {
            let handle = editor::EditorHandle {
                window: editor_window.clone_strong(),
                state: editor_state.clone(),
            };
            editor::open(&handle, &shared, idx as usize, None);
            crate::refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_lcd_duplicate_template(move |idx| {
            duplicate_current_template(&shared, idx as usize);
            crate::refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_lcd_delete_template(move |idx| {
            delete_current_template(&shared, idx as usize);
            crate::refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        let editor_window = editor.window.clone_strong();
        let editor_state = editor.state.clone();
        window.on_lcd_edit_template(move |idx| {
            let (starting_template, target_idx) = {
                let state = shared.lock().unwrap();
                let lcd = state.config.as_ref().and_then(|c| c.lcds.get(idx as usize));
                let current_id = lcd.and_then(|l| l.template_id.clone());
                let source = current_id
                    .as_ref()
                    .and_then(|id| state.lcd_templates.iter().find(|t| &t.id == id).cloned());
                (source, idx as usize)
            };

            let handle = editor::EditorHandle {
                window: editor_window.clone_strong(),
                state: editor_state.clone(),
            };
            editor::open(&handle, &shared, target_idx, starting_template);
            let _ = weak;
        });
    }

    {
        let shared = shared.clone();
        let browser_window = browser.window.clone_strong();
        let browser_catalog = browser.catalog.clone();
        window.on_lcd_browse_templates(move || {
            let handle = template_browser::BrowserHandle {
                window: browser_window.clone_strong(),
                catalog: browser_catalog.clone(),
            };
            template_browser::open(&handle, &shared);
        });
    }
}

fn default_sensor() -> lianli_shared::media::SensorDescriptor {
    use lianli_shared::media::{SensorRange, SensorSourceConfig};
    lianli_shared::media::SensorDescriptor {
        label: "CPU".to_string(),
        unit: "\u{00B0}C".to_string(),
        source: detect_cpu_temp_source().unwrap_or(SensorSourceConfig::Hwmon {
            name: String::new(),
            label: String::new(),
            device_path: String::new(),
        }),
        text_color: [255, 255, 255],
        background_color: [0, 0, 0],
        gauge_background_color: [0, 0, 0],
        gauge_ranges: vec![
            SensorRange {
                max: Some(50.0),
                color: [0, 200, 0],
                alpha: 255,
            },
            SensorRange {
                max: Some(80.0),
                color: [220, 140, 0],
                alpha: 255,
            },
            SensorRange {
                max: None,
                color: [220, 0, 0],
                alpha: 255,
            },
        ],
        update_interval_ms: 0, // legacy field, see SensorDescriptor docs
        gauge_start_angle: 60.0,
        gauge_sweep_angle: 300.0,
        gauge_outer_radius: 180.0,
        gauge_thickness: 40.0,
        bar_corner_radius: 20.0,
        value_font_size: 180.0,
        unit_font_size: 40.0,
        label_font_size: 40.0,
        font_path: detect_default_font(),
        decimal_places: 0,
        value_offset: -90,
        unit_offset: 80,
        label_offset: -140,
    }
}

fn detect_cpu_temp_source() -> Option<lianli_shared::media::SensorSourceConfig> {
    use lianli_shared::sensors::{enumerate_sensors, SensorSource, Unit};
    let cpu_temps: Vec<_> = enumerate_sensors()
        .into_iter()
        .filter(|s| {
            s.unit == Unit::C
                && s.sensor_name
                    .as_ref()
                    .map(|n| n.get_device_name() == "CPU")
                    .unwrap_or(false)
                && matches!(s.source, SensorSource::Hwmon { .. })
        })
        .collect();
    let picked = cpu_temps
        .iter()
        .find(|s| {
            s.sensor_name
                .as_ref()
                .map(|n| n.get_sensor_name() == "Control Temp")
                .unwrap_or(false)
        })
        .or_else(|| cpu_temps.first())?;
    match &picked.source {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => Some(lianli_shared::media::SensorSourceConfig::Hwmon {
            name: name.clone(),
            label: label.clone(),
            device_path: device_path.clone(),
        }),
        _ => None,
    }
}

fn detect_default_font() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ];
    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}
