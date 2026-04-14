use crate::EditorWidget;
use lianli_shared::fonts::{font_label_for_path, font_path_for_label, DEFAULT_FONT_LABEL};
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::sensors::{SensorInfo, SensorSource};
use lianli_shared::template::{BarOrientation, FontRef, ImageFit, TextAlign, Widget, WidgetKind};
use slint::{ModelRc, SharedString, VecModel};

pub(super) fn template_widgets_to_model(
    widgets: &[Widget],
    sensors: &[SensorInfo],
) -> ModelRc<EditorWidget> {
    let items: Vec<EditorWidget> = widgets
        .iter()
        .map(|w| widget_to_editor(w, sensors))
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub(super) fn sensor_index_for_source(source: &SensorSourceConfig, sensors: &[SensorInfo]) -> i32 {
    match source {
        SensorSourceConfig::Constant { .. } => 0,
        SensorSourceConfig::Command { .. } => sensors.len() as i32,
        _ => {
            let target = source.to_sensor_source();
            sensors
                .iter()
                .position(|s| s.source == target)
                .map(|i| i as i32)
                .unwrap_or(0)
        }
    }
}

pub(super) fn command_text_for_source(source: &SensorSourceConfig) -> SharedString {
    match source {
        SensorSourceConfig::Command { cmd } => SharedString::from(cmd.as_str()),
        _ => SharedString::default(),
    }
}

pub(super) fn widget_to_editor(w: &Widget, sensors: &[SensorInfo]) -> EditorWidget {
    let kind_str = w.kind.kind_id();
    let kind_label = WidgetKind::friendly_name_for(kind_str);
    let mut out = EditorWidget {
        id: SharedString::from(w.id.as_str()),
        kind: SharedString::from(kind_str),
        kind_label: SharedString::from(kind_label),
        x: w.x,
        y: w.y,
        width: w.width,
        height: w.height,
        rotation: w.rotation,
        visible: w.visible,
        update_interval_ms: w.update_interval_ms.unwrap_or(1000) as i32,
        text: SharedString::default(),
        font_name: SharedString::from(DEFAULT_FONT_LABEL),
        font_size: 32.0,
        color_r: 255,
        color_g: 255,
        color_b: 255,
        color_a: 255,
        align: SharedString::from("center"),
        format: SharedString::from("{:.0}"),
        unit: SharedString::default(),
        source_index: 0,
        command: SharedString::default(),
        value_min: 0.0,
        value_max: 100.0,
        start_angle: 0.0,
        sweep_angle: 270.0,
        ring_thickness_pct: 22,
        bg_r: 40,
        bg_g: 40,
        bg_b: 40,
        bg_a: 255,
        tick_count: 10,
        show_gauge: true,
        show_needle: true,
        needle_width: 14.0,
        needle_length_pct: 95,
        needle_color_r: 255,
        needle_color_g: 255,
        needle_color_b: 255,
        needle_color_a: 255,
        tick_color_r: 120,
        tick_color_g: 140,
        tick_color_b: 160,
        tick_color_a: 255,
        needle_border_r: 174,
        needle_border_g: 10,
        needle_border_b: 16,
        needle_border_a: 255,
        needle_border_width: 1.5,
        show_labels: true,
        image_path: SharedString::default(),
        opacity: 1.0,
        fps: w.fps.unwrap_or(30.0),
        corner_radius: 0,
        bg_corner_radius: 0,
        value_corner_radius: 0,
        letter_spacing: 0,
        clock_show_seconds: true,
        clock_show_hour_ticks: true,
        clock_show_minor_ticks: true,
        clock_show_numbers: false,
        clock_second_hand_r: 220,
        clock_second_hand_g: 40,
        clock_second_hand_b: 40,
        clock_second_hand_a: 255,
        clock_minor_tick_r: 220,
        clock_minor_tick_g: 220,
        clock_minor_tick_b: 220,
        clock_minor_tick_a: 255,
        clock_hub_r: 240,
        clock_hub_g: 240,
        clock_hub_b: 240,
        clock_hub_a: 255,
        clock_hour_hand_width: 6,
        clock_minute_hand_width: 4,
        clock_second_hand_width: 2,
        clock_hour_length_pct: 55,
        clock_minute_length_pct: 80,
        clock_second_length_pct: 90,
        clock_hour_tick_length_pct: 12,
        clock_minor_tick_length_pct: 5,
        clock_hour_tick_width: 3,
        clock_minor_tick_width: 2,
        clock_hub_radius: 6,
    };
    match &w.kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
            letter_spacing,
        } => {
            out.text = SharedString::from(text.as_str());
            out.font_name = SharedString::from(font_ref_to_label(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.color_a = color[3] as i32;
            out.align = SharedString::from(text_align_name(*align));
            out.letter_spacing = letter_spacing.round() as i32;
        }
        WidgetKind::ValueText {
            source,
            format,
            unit,
            font,
            font_size,
            color,
            align,
            value_min,
            value_max,
            letter_spacing,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.format = SharedString::from(format.as_str());
            out.unit = SharedString::from(unit.as_str());
            out.font_name = SharedString::from(font_ref_to_label(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.color_a = color[3] as i32;
            out.align = SharedString::from(text_align_name(*align));
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.letter_spacing = letter_spacing.round() as i32;
        }
        WidgetKind::RadialGauge {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            bg_corner_radius,
            value_corner_radius,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.ring_thickness_pct = ((1.0 - inner_radius_pct.clamp(0.0, 0.99)) * 100.0)
                .round()
                .clamp(1.0, 100.0) as i32;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.bg_corner_radius = bg_corner_radius.max(0.0).round() as i32;
            out.value_corner_radius = value_corner_radius.max(0.0).round() as i32;
        }
        WidgetKind::VerticalBar {
            source,
            value_min,
            value_max,
            background_color,
            corner_radius,
            ..
        }
        | WidgetKind::HorizontalBar {
            source,
            value_min,
            value_max,
            background_color,
            corner_radius,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.corner_radius = corner_radius.round() as i32;
        }
        WidgetKind::Speedometer {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            background_color,
            tick_count,
            show_gauge,
            show_needle,
            needle_width,
            needle_length_pct,
            needle_border_color,
            needle_border_width,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.tick_count = *tick_count as i32;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.show_gauge = *show_gauge;
            out.show_needle = *show_needle;
            out.needle_width = *needle_width;
            out.needle_length_pct = (*needle_length_pct * 100.0).round() as i32;
            out.needle_color_r = needle_color[0] as i32;
            out.needle_color_g = needle_color[1] as i32;
            out.needle_color_b = needle_color[2] as i32;
            out.needle_color_a = needle_color[3] as i32;
            out.tick_color_r = tick_color[0] as i32;
            out.tick_color_g = tick_color[1] as i32;
            out.tick_color_b = tick_color[2] as i32;
            out.tick_color_a = tick_color[3] as i32;
            out.needle_border_r = needle_border_color[0] as i32;
            out.needle_border_g = needle_border_color[1] as i32;
            out.needle_border_b = needle_border_color[2] as i32;
            out.needle_border_a = needle_border_color[3] as i32;
            out.needle_border_width = *needle_border_width;
        }
        WidgetKind::CoreBars {
            background_color,
            show_labels,
            ..
        } => {
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.show_labels = *show_labels;
        }
        WidgetKind::Image { path, opacity, .. } => {
            out.image_path = SharedString::from(path.display().to_string());
            out.opacity = *opacity;
        }
        WidgetKind::Video { path, opacity, .. } => {
            out.image_path = SharedString::from(path.display().to_string());
            out.opacity = *opacity;
        }
        WidgetKind::ClockDigital {
            format,
            font,
            font_size,
            color,
            align,
            letter_spacing,
        } => {
            out.format = SharedString::from(format.as_str());
            out.font_name = SharedString::from(font_ref_to_label(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.color_a = color[3] as i32;
            out.align = SharedString::from(text_align_name(*align));
            out.letter_spacing = letter_spacing.round() as i32;
        }
        WidgetKind::ClockAnalog {
            face_color,
            tick_color,
            minor_tick_color,
            hour_hand_color,
            minute_hand_color,
            second_hand_color,
            hub_color,
            numbers_color,
            numbers_font,
            numbers_font_size,
            show_seconds,
            show_hour_ticks,
            show_minor_ticks,
            show_numbers,
            hour_hand_width,
            minute_hand_width,
            second_hand_width,
            hour_hand_length_pct,
            minute_hand_length_pct,
            second_hand_length_pct,
            hour_tick_length_pct,
            minor_tick_length_pct,
            hour_tick_width,
            minor_tick_width,
            hub_radius,
        } => {
            out.bg_r = face_color[0] as i32;
            out.bg_g = face_color[1] as i32;
            out.bg_b = face_color[2] as i32;
            out.bg_a = face_color[3] as i32;
            out.tick_color_r = tick_color[0] as i32;
            out.tick_color_g = tick_color[1] as i32;
            out.tick_color_b = tick_color[2] as i32;
            out.tick_color_a = tick_color[3] as i32;
            out.clock_minor_tick_r = minor_tick_color[0] as i32;
            out.clock_minor_tick_g = minor_tick_color[1] as i32;
            out.clock_minor_tick_b = minor_tick_color[2] as i32;
            out.clock_minor_tick_a = minor_tick_color[3] as i32;
            out.needle_color_r = hour_hand_color[0] as i32;
            out.needle_color_g = hour_hand_color[1] as i32;
            out.needle_color_b = hour_hand_color[2] as i32;
            out.needle_color_a = hour_hand_color[3] as i32;
            out.needle_border_r = minute_hand_color[0] as i32;
            out.needle_border_g = minute_hand_color[1] as i32;
            out.needle_border_b = minute_hand_color[2] as i32;
            out.needle_border_a = minute_hand_color[3] as i32;
            out.clock_second_hand_r = second_hand_color[0] as i32;
            out.clock_second_hand_g = second_hand_color[1] as i32;
            out.clock_second_hand_b = second_hand_color[2] as i32;
            out.clock_second_hand_a = second_hand_color[3] as i32;
            out.clock_hub_r = hub_color[0] as i32;
            out.clock_hub_g = hub_color[1] as i32;
            out.clock_hub_b = hub_color[2] as i32;
            out.clock_hub_a = hub_color[3] as i32;
            out.color_r = numbers_color[0] as i32;
            out.color_g = numbers_color[1] as i32;
            out.color_b = numbers_color[2] as i32;
            out.color_a = numbers_color[3] as i32;
            out.font_name = SharedString::from(font_ref_to_label(numbers_font));
            out.font_size = *numbers_font_size;
            out.clock_show_seconds = *show_seconds;
            out.clock_show_hour_ticks = *show_hour_ticks;
            out.clock_show_minor_ticks = *show_minor_ticks;
            out.clock_show_numbers = *show_numbers;
            out.clock_hour_hand_width = hour_hand_width.round() as i32;
            out.clock_minute_hand_width = minute_hand_width.round() as i32;
            out.clock_second_hand_width = second_hand_width.round() as i32;
            out.clock_hour_length_pct = (hour_hand_length_pct * 100.0).round() as i32;
            out.clock_minute_length_pct = (minute_hand_length_pct * 100.0).round() as i32;
            out.clock_second_length_pct = (second_hand_length_pct * 100.0).round() as i32;
            out.clock_hour_tick_length_pct = (hour_tick_length_pct * 100.0).round() as i32;
            out.clock_minor_tick_length_pct = (minor_tick_length_pct * 100.0).round() as i32;
            out.clock_hour_tick_width = hour_tick_width.round() as i32;
            out.clock_minor_tick_width = minor_tick_width.round() as i32;
            out.clock_hub_radius = hub_radius.round() as i32;
        }
    }
    out
}

pub(super) fn font_ref_to_label(f: &FontRef) -> String {
    font_label_for_path(f.path.as_deref())
}

pub(super) fn label_to_font_ref(label: &str) -> FontRef {
    FontRef {
        path: font_path_for_label(label),
    }
}

pub(super) fn text_align_name(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}

pub(super) fn make_default_widget(id: &str, kind_str: &str, cx: f32, cy: f32) -> Widget {
    let kind = match kind_str {
        "label" => WidgetKind::Label {
            text: "Label".into(),
            font: FontRef::default(),
            font_size: 32.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            letter_spacing: 0.0,
        },
        "value_text" => WidgetKind::ValueText {
            source: SensorSourceConfig::CpuUsage,
            format: "{:.0}".into(),
            unit: "%".into(),
            font: FontRef::default(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            value_min: 0.0,
            value_max: 100.0,
            ranges: default_ranges(),
            letter_spacing: 0.0,
        },
        "radial_gauge" => WidgetKind::RadialGauge {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 135.0,
            sweep_angle: 270.0,
            inner_radius_pct: 0.78,
            background_color: [40, 40, 40, 255],
            ranges: default_ranges(),
            bg_corner_radius: 0.0,
            value_corner_radius: 0.0,
        },
        "vertical_bar" => WidgetKind::VerticalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40, 255],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "horizontal_bar" => WidgetKind::HorizontalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40, 255],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "speedometer" => WidgetKind::Speedometer {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 180.0,
            sweep_angle: 180.0,
            needle_color: [255, 255, 255, 255],
            tick_color: [120, 140, 160, 255],
            tick_count: 10,
            background_color: [40, 40, 40, 255],
            ranges: default_ranges(),
            show_gauge: true,
            show_needle: true,
            needle_width: 14.0,
            needle_length_pct: 0.95,
            needle_border_color: [174, 10, 16, 255],
            needle_border_width: 1.5,
        },
        "core_bars" => WidgetKind::CoreBars {
            orientation: BarOrientation::Horizontal,
            background_color: [30, 30, 30, 255],
            show_labels: true,
            ranges: default_ranges(),
        },
        "image" => WidgetKind::Image {
            path: std::path::PathBuf::new(),
            opacity: 1.0,
            fit: ImageFit::Stretch,
        },
        "video" => WidgetKind::Video {
            path: std::path::PathBuf::new(),
            loop_playback: true,
            opacity: 1.0,
            fit: ImageFit::Stretch,
        },
        "clock_digital" => WidgetKind::ClockDigital {
            format: "%H:%M".to_string(),
            font: FontRef::default(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            letter_spacing: 0.0,
        },
        "clock_analog" => WidgetKind::ClockAnalog {
            face_color: [30, 30, 30, 255],
            tick_color: [220, 220, 220, 255],
            minor_tick_color: [220, 220, 220, 255],
            hour_hand_color: [240, 240, 240, 255],
            minute_hand_color: [240, 240, 240, 255],
            second_hand_color: [220, 40, 40, 255],
            hub_color: [240, 240, 240, 255],
            numbers_color: [230, 230, 230, 255],
            numbers_font: FontRef::default(),
            numbers_font_size: 24.0,
            show_seconds: true,
            show_hour_ticks: true,
            show_minor_ticks: true,
            show_numbers: false,
            hour_hand_width: 6.0,
            minute_hand_width: 4.0,
            second_hand_width: 2.0,
            hour_hand_length_pct: 0.55,
            minute_hand_length_pct: 0.8,
            second_hand_length_pct: 0.9,
            hour_tick_length_pct: 0.12,
            minor_tick_length_pct: 0.05,
            hour_tick_width: 3.0,
            minor_tick_width: 1.5,
            hub_radius: 6.0,
        },
        _ => WidgetKind::Label {
            text: "Label".into(),
            font: FontRef::default(),
            font_size: 32.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            letter_spacing: 0.0,
        },
    };
    Widget {
        id: id.to_string(),
        kind,
        x: cx,
        y: cy,
        width: 120.0,
        height: 80.0,
        rotation: 0.0,
        visible: true,
        update_interval_ms: None,
        fps: None,
        sensor_category: None,
    }
}

pub(super) fn default_ranges() -> Vec<SensorRange> {
    vec![
        SensorRange {
            max: Some(50.0),
            color: [0, 200, 0],
            alpha: 255,
        },
        SensorRange {
            max: Some(75.0),
            color: [220, 140, 0],
            alpha: 255,
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
            alpha: 255,
        },
    ]
}

pub(super) fn parse_u8(s: &str) -> u8 {
    s.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8
}

pub(super) fn parse_align(s: &str) -> TextAlign {
    match s {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

pub(super) fn parse_sensor_source(
    label: &str,
    sensors: &[SensorInfo],
) -> Option<SensorSourceConfig> {
    if label.ends_with(". Custom command") || label == "Custom command" {
        return Some(SensorSourceConfig::Command { cmd: String::new() });
    }
    let idx: usize = label.split('.').next()?.parse().ok()?;
    if idx == 0 {
        return None;
    }
    let sensor = sensors.get(idx - 1)?;
    Some(match &sensor.source {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name: name.clone(),
            label: label.clone(),
            device_path: device_path.clone(),
        },
        SensorSource::NvidiaGpu { gpu_index, metric } => SensorSourceConfig::NvidiaGpu {
            gpu_index: *gpu_index,
            metric: *metric,
        },
        SensorSource::AmdGpuUsage { card_index } => SensorSourceConfig::AmdGpuUsage {
            card_index: *card_index,
        },
        SensorSource::WirelessCoolant { device_id } => SensorSourceConfig::WirelessCoolant {
            device_id: device_id.clone(),
        },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd: cmd.clone() },
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
    })
}

pub(super) fn blank_editor_widget() -> EditorWidget {
    EditorWidget {
        id: SharedString::default(),
        kind: SharedString::default(),
        kind_label: SharedString::default(),
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        rotation: 0.0,
        visible: true,
        update_interval_ms: 1000,
        text: SharedString::default(),
        font_name: SharedString::from(DEFAULT_FONT_LABEL),
        font_size: 32.0,
        color_r: 255,
        color_g: 255,
        color_b: 255,
        color_a: 255,
        align: SharedString::from("center"),
        format: SharedString::from("{:.0}"),
        unit: SharedString::default(),
        source_index: 0,
        command: SharedString::default(),
        value_min: 0.0,
        value_max: 100.0,
        start_angle: 0.0,
        sweep_angle: 270.0,
        ring_thickness_pct: 22,
        bg_r: 40,
        bg_g: 40,
        bg_b: 40,
        bg_a: 255,
        tick_count: 10,
        show_gauge: true,
        show_needle: true,
        needle_width: 14.0,
        needle_length_pct: 95,
        needle_color_r: 255,
        needle_color_g: 255,
        needle_color_b: 255,
        needle_color_a: 255,
        tick_color_r: 120,
        tick_color_g: 140,
        tick_color_b: 160,
        tick_color_a: 255,
        needle_border_r: 174,
        needle_border_g: 10,
        needle_border_b: 16,
        needle_border_a: 255,
        needle_border_width: 1.5,
        show_labels: true,
        image_path: SharedString::default(),
        opacity: 1.0,
        fps: 30.0,
        letter_spacing: 0,
        clock_show_seconds: true,
        clock_show_hour_ticks: true,
        clock_show_minor_ticks: true,
        clock_show_numbers: false,
        clock_second_hand_r: 220,
        clock_second_hand_g: 40,
        clock_second_hand_b: 40,
        clock_second_hand_a: 255,
        clock_minor_tick_r: 220,
        clock_minor_tick_g: 220,
        clock_minor_tick_b: 220,
        clock_minor_tick_a: 255,
        clock_hub_r: 240,
        clock_hub_g: 240,
        clock_hub_b: 240,
        clock_hub_a: 255,
        clock_hour_hand_width: 6,
        clock_minute_hand_width: 4,
        clock_second_hand_width: 2,
        clock_hour_length_pct: 55,
        clock_minute_length_pct: 80,
        clock_second_length_pct: 90,
        clock_hour_tick_length_pct: 12,
        clock_minor_tick_length_pct: 5,
        clock_hour_tick_width: 3,
        clock_minor_tick_width: 2,
        clock_hub_radius: 6,
        corner_radius: 0,
        bg_corner_radius: 0,
        value_corner_radius: 0,
    }
}
