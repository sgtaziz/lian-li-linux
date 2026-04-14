use crate::EditorRange;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::{Widget, WidgetKind};
use slint::{ModelRc, VecModel};

pub(super) fn apply_widget_field(
    widget: &mut Widget,
    field: &str,
    val: &str,
    sensors: &[SensorInfo],
) {
    match field {
        "id" => {
            if !val.trim().is_empty() {
                widget.id = val.trim().to_string();
            }
        }
        "x" => {
            if let Ok(v) = val.parse() {
                widget.x = v;
            }
        }
        "y" => {
            if let Ok(v) = val.parse() {
                widget.y = v;
            }
        }
        "width" => {
            if let Ok(v) = val.parse() {
                widget.width = v;
            }
        }
        "height" => {
            if let Ok(v) = val.parse() {
                widget.height = v;
            }
        }
        "rotation" => {
            if let Ok(v) = val.parse() {
                widget.rotation = v;
            }
        }
        "visible" => widget.visible = val == "true",
        "update_interval_ms" => {
            if let Ok(v) = val.parse::<u64>() {
                widget.update_interval_ms = Some(v.clamp(100, 10_000));
            }
        }
        "fps" => {
            if let Ok(v) = val.parse::<f32>() {
                widget.fps = Some(v);
            }
        }
        _ => apply_kind_field(&mut widget.kind, field, val, sensors),
    }
}

pub(super) fn apply_kind_field(
    kind: &mut WidgetKind,
    field: &str,
    val: &str,
    sensors: &[SensorInfo],
) {
    match kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
            letter_spacing,
        } => match field {
            "text" => *text = val.to_string(),
            "font" => *font = super::mapping::label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = super::mapping::parse_u8(val),
            "color_g" => color[1] = super::mapping::parse_u8(val),
            "color_b" => color[2] = super::mapping::parse_u8(val),
            "color_a" => color[3] = super::mapping::parse_u8(val),
            "align" => *align = super::mapping::parse_align(val),
            "letter_spacing" => {
                if let Ok(v) = val.parse::<f32>() {
                    *letter_spacing = v;
                }
            }
            _ => {}
        },
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
        } => match field {
            "text" => {}
            "format" => *format = val.to_string(),
            "unit" => *unit = val.to_string(),
            "font" => *font = super::mapping::label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = super::mapping::parse_u8(val),
            "color_g" => color[1] = super::mapping::parse_u8(val),
            "color_b" => color[2] = super::mapping::parse_u8(val),
            "color_a" => color[3] = super::mapping::parse_u8(val),
            "align" => *align = super::mapping::parse_align(val),
            "source" => {
                if let Some(new) = super::mapping::parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "letter_spacing" => {
                if let Ok(v) = val.parse::<f32>() {
                    *letter_spacing = v;
                }
            }
            _ => {}
        },
        WidgetKind::RadialGauge {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ranges: _,
            bg_corner_radius,
            value_corner_radius,
        } => match field {
            "source" => {
                if let Some(new) = super::mapping::parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "start_angle" => {
                if let Ok(v) = val.parse() {
                    *start_angle = v;
                }
            }
            "sweep_angle" => {
                if let Ok(v) = val.parse() {
                    *sweep_angle = v;
                }
            }
            "ring_thickness_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *inner_radius_pct = 1.0 - (v.clamp(1, 100) as f32) / 100.0;
                }
            }
            "bg_r" => background_color[0] = super::mapping::parse_u8(val),
            "bg_g" => background_color[1] = super::mapping::parse_u8(val),
            "bg_b" => background_color[2] = super::mapping::parse_u8(val),
            "bg_a" => background_color[3] = super::mapping::parse_u8(val),
            "bg_corner_radius" => {
                if let Ok(v) = val.parse::<f32>() {
                    *bg_corner_radius = v.max(0.0);
                }
            }
            "value_corner_radius" => {
                if let Ok(v) = val.parse::<f32>() {
                    *value_corner_radius = v.max(0.0);
                }
            }
            _ => {}
        },
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
        } => match field {
            "source" => {
                if let Some(new) = super::mapping::parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "bg_r" => background_color[0] = super::mapping::parse_u8(val),
            "bg_g" => background_color[1] = super::mapping::parse_u8(val),
            "bg_b" => background_color[2] = super::mapping::parse_u8(val),
            "bg_a" => background_color[3] = super::mapping::parse_u8(val),
            "corner_radius" => {
                if let Ok(v) = val.parse::<f32>() {
                    *corner_radius = v.max(0.0);
                }
            }
            _ => {}
        },
        WidgetKind::Speedometer {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            background_color,
            show_gauge,
            show_needle,
            needle_width,
            needle_length_pct,
            needle_border_color,
            needle_border_width,
            ..
        } => match field {
            "source" => {
                if let Some(new) = super::mapping::parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "start_angle" => {
                if let Ok(v) = val.parse() {
                    *start_angle = v;
                }
            }
            "sweep_angle" => {
                if let Ok(v) = val.parse() {
                    *sweep_angle = v;
                }
            }
            "show_gauge" => *show_gauge = val == "true",
            "show_needle" => *show_needle = val == "true",
            "needle_width" => {
                if let Ok(v) = val.parse() {
                    *needle_width = v;
                }
            }
            "needle_length_pct" => {
                if let Ok(v) = val.parse::<f32>() {
                    *needle_length_pct = (v / 100.0).clamp(0.1, 1.5);
                }
            }
            "needle_color_r" => needle_color[0] = super::mapping::parse_u8(val),
            "needle_color_g" => needle_color[1] = super::mapping::parse_u8(val),
            "needle_color_b" => needle_color[2] = super::mapping::parse_u8(val),
            "needle_color_a" => needle_color[3] = super::mapping::parse_u8(val),
            "tick_color_r" => tick_color[0] = super::mapping::parse_u8(val),
            "tick_color_g" => tick_color[1] = super::mapping::parse_u8(val),
            "tick_color_b" => tick_color[2] = super::mapping::parse_u8(val),
            "tick_color_a" => tick_color[3] = super::mapping::parse_u8(val),
            "needle_border_r" => needle_border_color[0] = super::mapping::parse_u8(val),
            "needle_border_g" => needle_border_color[1] = super::mapping::parse_u8(val),
            "needle_border_b" => needle_border_color[2] = super::mapping::parse_u8(val),
            "needle_border_a" => needle_border_color[3] = super::mapping::parse_u8(val),
            "needle_border_width" => {
                if let Ok(v) = val.parse() {
                    *needle_border_width = v;
                }
            }
            "bg_r" => background_color[0] = super::mapping::parse_u8(val),
            "bg_g" => background_color[1] = super::mapping::parse_u8(val),
            "bg_b" => background_color[2] = super::mapping::parse_u8(val),
            "bg_a" => background_color[3] = super::mapping::parse_u8(val),
            _ => {}
        },
        WidgetKind::CoreBars {
            background_color,
            show_labels,
            ..
        } => match field {
            "show_labels" => *show_labels = val == "true",
            "bg_r" => background_color[0] = super::mapping::parse_u8(val),
            "bg_g" => background_color[1] = super::mapping::parse_u8(val),
            "bg_b" => background_color[2] = super::mapping::parse_u8(val),
            "bg_a" => background_color[3] = super::mapping::parse_u8(val),
            _ => {}
        },
        WidgetKind::Image { path, .. } | WidgetKind::Video { path, .. } => match field {
            "path" => *path = std::path::PathBuf::from(val),
            _ => {}
        },
        WidgetKind::ClockDigital {
            format,
            font,
            font_size,
            color,
            align,
            letter_spacing,
        } => match field {
            "format" => *format = val.to_string(),
            "font" => *font = super::mapping::label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = super::mapping::parse_u8(val),
            "color_g" => color[1] = super::mapping::parse_u8(val),
            "color_b" => color[2] = super::mapping::parse_u8(val),
            "color_a" => color[3] = super::mapping::parse_u8(val),
            "align" => *align = super::mapping::parse_align(val),
            "letter_spacing" => {
                if let Ok(v) = val.parse::<f32>() {
                    *letter_spacing = v;
                }
            }
            _ => {}
        },
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
        } => match field {
            "bg_r" => face_color[0] = super::mapping::parse_u8(val),
            "bg_g" => face_color[1] = super::mapping::parse_u8(val),
            "bg_b" => face_color[2] = super::mapping::parse_u8(val),
            "bg_a" => face_color[3] = super::mapping::parse_u8(val),
            "tick_color_r" => tick_color[0] = super::mapping::parse_u8(val),
            "tick_color_g" => tick_color[1] = super::mapping::parse_u8(val),
            "tick_color_b" => tick_color[2] = super::mapping::parse_u8(val),
            "tick_color_a" => tick_color[3] = super::mapping::parse_u8(val),
            "clock_minor_tick_r" => minor_tick_color[0] = super::mapping::parse_u8(val),
            "clock_minor_tick_g" => minor_tick_color[1] = super::mapping::parse_u8(val),
            "clock_minor_tick_b" => minor_tick_color[2] = super::mapping::parse_u8(val),
            "clock_minor_tick_a" => minor_tick_color[3] = super::mapping::parse_u8(val),
            "needle_color_r" => hour_hand_color[0] = super::mapping::parse_u8(val),
            "needle_color_g" => hour_hand_color[1] = super::mapping::parse_u8(val),
            "needle_color_b" => hour_hand_color[2] = super::mapping::parse_u8(val),
            "needle_color_a" => hour_hand_color[3] = super::mapping::parse_u8(val),
            "needle_border_r" => minute_hand_color[0] = super::mapping::parse_u8(val),
            "needle_border_g" => minute_hand_color[1] = super::mapping::parse_u8(val),
            "needle_border_b" => minute_hand_color[2] = super::mapping::parse_u8(val),
            "needle_border_a" => minute_hand_color[3] = super::mapping::parse_u8(val),
            "clock_second_hand_r" => second_hand_color[0] = super::mapping::parse_u8(val),
            "clock_second_hand_g" => second_hand_color[1] = super::mapping::parse_u8(val),
            "clock_second_hand_b" => second_hand_color[2] = super::mapping::parse_u8(val),
            "clock_second_hand_a" => second_hand_color[3] = super::mapping::parse_u8(val),
            "clock_hub_r" => hub_color[0] = super::mapping::parse_u8(val),
            "clock_hub_g" => hub_color[1] = super::mapping::parse_u8(val),
            "clock_hub_b" => hub_color[2] = super::mapping::parse_u8(val),
            "clock_hub_a" => hub_color[3] = super::mapping::parse_u8(val),
            "color_r" => numbers_color[0] = super::mapping::parse_u8(val),
            "color_g" => numbers_color[1] = super::mapping::parse_u8(val),
            "color_b" => numbers_color[2] = super::mapping::parse_u8(val),
            "color_a" => numbers_color[3] = super::mapping::parse_u8(val),
            "font" => *numbers_font = super::mapping::label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *numbers_font_size = v;
                }
            }
            "clock_show_seconds" => *show_seconds = val == "true",
            "clock_show_hour_ticks" => *show_hour_ticks = val == "true",
            "clock_show_minor_ticks" => *show_minor_ticks = val == "true",
            "clock_show_numbers" => *show_numbers = val == "true",
            "clock_hour_hand_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    *hour_hand_width = v.max(1.0);
                }
            }
            "clock_minute_hand_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    *minute_hand_width = v.max(1.0);
                }
            }
            "clock_second_hand_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    *second_hand_width = v.max(1.0);
                }
            }
            "clock_hour_length_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *hour_hand_length_pct = (v.clamp(10, 120) as f32) / 100.0;
                }
            }
            "clock_minute_length_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *minute_hand_length_pct = (v.clamp(10, 120) as f32) / 100.0;
                }
            }
            "clock_second_length_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *second_hand_length_pct = (v.clamp(10, 120) as f32) / 100.0;
                }
            }
            "clock_hour_tick_length_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *hour_tick_length_pct = (v.clamp(0, 50) as f32) / 100.0;
                }
            }
            "clock_minor_tick_length_pct" => {
                if let Ok(v) = val.parse::<i32>() {
                    *minor_tick_length_pct = (v.clamp(0, 50) as f32) / 100.0;
                }
            }
            "clock_hour_tick_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    *hour_tick_width = v.max(1.0);
                }
            }
            "clock_minor_tick_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    *minor_tick_width = v.max(1.0);
                }
            }
            "clock_hub_radius" => {
                if let Ok(v) = val.parse::<f32>() {
                    *hub_radius = v.max(0.0);
                }
            }
            _ => {}
        },
    }
}

pub(super) fn widget_ranges_mut(kind: &mut WidgetKind) -> Option<&mut Vec<SensorRange>> {
    match kind {
        WidgetKind::RadialGauge { ranges, .. }
        | WidgetKind::VerticalBar { ranges, .. }
        | WidgetKind::HorizontalBar { ranges, .. }
        | WidgetKind::Speedometer { ranges, .. }
        | WidgetKind::CoreBars { ranges, .. }
        | WidgetKind::ValueText { ranges, .. } => Some(ranges),
        _ => None,
    }
}

pub(super) fn widget_ranges(kind: &WidgetKind) -> Option<&[SensorRange]> {
    match kind {
        WidgetKind::RadialGauge { ranges, .. }
        | WidgetKind::VerticalBar { ranges, .. }
        | WidgetKind::HorizontalBar { ranges, .. }
        | WidgetKind::Speedometer { ranges, .. }
        | WidgetKind::CoreBars { ranges, .. }
        | WidgetKind::ValueText { ranges, .. } => Some(ranges.as_slice()),
        _ => None,
    }
}

pub(super) fn apply_range_field(range: &mut SensorRange, field: &str, val: &str) {
    match field {
        "max" => {
            if let Ok(v) = val.parse::<i32>() {
                range.max = if v < 0 {
                    None
                } else {
                    Some((v.clamp(0, 100)) as f32)
                };
            }
        }
        "color_r" => range.color[0] = super::mapping::parse_u8(val),
        "color_g" => range.color[1] = super::mapping::parse_u8(val),
        "color_b" => range.color[2] = super::mapping::parse_u8(val),
        "color_a" => range.alpha = super::mapping::parse_u8(val),
        _ => {}
    }
}

pub(super) fn ranges_to_editor(ranges: &[SensorRange]) -> ModelRc<EditorRange> {
    let items: Vec<EditorRange> = ranges
        .iter()
        .map(|r| EditorRange {
            max_pct: r.max.map(|v| v as i32).unwrap_or(-1),
            color_r: r.color[0] as i32,
            color_g: r.color[1] as i32,
            color_b: r.color[2] as i32,
            color_a: r.alpha as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}
