use lianli_shared::fonts::font_path_for_label;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::sensors::{SensorInfo, SensorSource};
use lianli_shared::template::{
    default_gradient_stops, BarOrientation, FontRef, ImageFit, TextAlign, Widget, WidgetKind,
};

pub(in crate::editor) fn label_to_font_ref(label: &str) -> FontRef {
    FontRef {
        path: font_path_for_label(label),
    }
}

pub(in crate::editor) fn make_default_widget(id: &str, kind_str: &str, cx: f32, cy: f32) -> Widget {
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
            gradient: false,
            gradient_stops: default_gradient_stops(),
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
        "sparkline" => WidgetKind::Sparkline {
            source: SensorSourceConfig::Constant { value: 0.0 },
            value_min: 0.0,
            value_max: 100.0,
            auto_range: false,
            history_length: 60,
            line_width: 2.0,
            line_color: [80, 180, 240, 255],
            fill_color: [80, 180, 240, 80],
            fill_from_ranges: false,
            range_blend: false,
            background_color: [30, 30, 30, 255],
            ranges: Vec::new(),
            border_color: [80, 90, 110, 255],
            border_width: 0.0,
            corner_radius: 0.0,
            padding: 4.0,
            show_points: false,
            point_radius: 2.5,
            show_baseline: false,
            baseline_value: 0.0,
            baseline_color: [140, 140, 160, 160],
            baseline_width: 1.0,
            smooth: false,
            scroll_rtl: false,
            show_gridlines: false,
            gridlines_horizontal: 3,
            gridlines_vertical: 0,
            gridline_color: [120, 120, 140, 90],
            gridline_width: 1.0,
            show_axis_labels: false,
            axis_label_count: 3,
            axis_labels_on_right: false,
            axis_label_format: "{:.0}".to_string(),
            axis_label_font: FontRef::default(),
            axis_label_size: 11.0,
            axis_label_color: [200, 200, 210, 220],
            axis_label_padding: 4.0,
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

pub(in crate::editor) fn default_ranges() -> Vec<SensorRange> {
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

pub(in crate::editor) fn parse_u8(s: &str) -> u8 {
    s.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8
}

pub(in crate::editor) fn parse_align(s: &str) -> TextAlign {
    match s {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

pub(in crate::editor) fn parse_sensor_source(
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
        SensorSource::NetworkRate { iface, direction } => match direction {
            lianli_shared::sensors::NetDirection::Rx => SensorSourceConfig::NetworkRx {
                iface: iface.clone(),
            },
            lianli_shared::sensors::NetDirection::Tx => SensorSourceConfig::NetworkTx {
                iface: iface.clone(),
            },
        },
        SensorSource::DiskRate { device, direction } => match direction {
            lianli_shared::sensors::DiskDirection::Read => SensorSourceConfig::DiskRead {
                device: device.clone(),
            },
            lianli_shared::sensors::DiskDirection::Write => SensorSourceConfig::DiskWrite {
                device: device.clone(),
            },
        },
    })
}
