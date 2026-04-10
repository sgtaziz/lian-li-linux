//! Built-in `LcdTemplate`s shipped with the software. Editing a built-in in
//! the GUI triggers a "Duplicate to edit" flow so the originals stay pristine
//! and survive software upgrades.

use crate::media::{SensorRange, SensorSourceConfig};
use crate::sensors::{find_default_cpu_temp, SensorInfo, SensorSource};
use crate::template::{
    BarOrientation, BuiltinAsset, BuiltinFont, FontRef, LcdTemplate, TemplateBackground, TextAlign,
    Widget, WidgetKind,
};

pub const BUILTIN_COOLER_ID: &str = "cooler-default";
pub const BUILTIN_DOUBLEGAUGE_ID: &str = "doublegauge-default";

pub fn is_builtin_id(id: &str) -> bool {
    id == BUILTIN_COOLER_ID || id == BUILTIN_DOUBLEGAUGE_ID
}

pub fn builtin_templates() -> Vec<LcdTemplate> {
    vec![builtin_cooler(), builtin_doublegauge()]
}

pub fn builtin_template(id: &str) -> Option<LcdTemplate> {
    match id {
        BUILTIN_COOLER_ID => Some(builtin_cooler()),
        BUILTIN_DOUBLEGAUGE_ID => Some(builtin_doublegauge()),
        _ => None,
    }
}

pub fn builtin_template_resolved(id: &str, sensors: &[SensorInfo]) -> Option<LcdTemplate> {
    let mut tpl = builtin_template(id)?;
    let Some(cpu_temp_source) = find_default_cpu_temp(sensors) else {
        return Some(tpl);
    };
    let cfg = sensor_source_to_config(&cpu_temp_source);
    for w in tpl.widgets.iter_mut() {
        if matches!(w.id.as_str(), "bar-temp" | "value-temp" | "gauge-temp") {
            if let Some(s) = widget_source_mut(&mut w.kind) {
                *s = cfg.clone();
            }
        }
    }
    Some(tpl)
}

fn sensor_source_to_config(s: &SensorSource) -> SensorSourceConfig {
    match s {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name: name.clone(),
            label: label.clone(),
            device_path: device_path.clone(),
        },
        SensorSource::NvidiaGpu { gpu_index } => SensorSourceConfig::NvidiaGpu {
            gpu_index: *gpu_index,
        },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd: cmd.clone() },
        SensorSource::WirelessCoolant { device_id } => SensorSourceConfig::WirelessCoolant {
            device_id: device_id.clone(),
        },
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
    }
}

fn widget_source_mut(kind: &mut WidgetKind) -> Option<&mut SensorSourceConfig> {
    match kind {
        WidgetKind::ValueText { source, .. }
        | WidgetKind::RadialGauge { source, .. }
        | WidgetKind::VerticalBar { source, .. }
        | WidgetKind::HorizontalBar { source, .. }
        | WidgetKind::Speedometer { source, .. } => Some(source),
        _ => None,
    }
}

fn widget(id: &str, kind: WidgetKind, x: f32, y: f32, width: f32, height: f32) -> Widget {
    Widget {
        id: id.to_string(),
        kind,
        x,
        y,
        width,
        height,
        rotation: 0.0,
        visible: true,
        update_interval_ms: None,
        fps: None,
    }
}

fn default_ranges() -> Vec<SensorRange> {
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

fn builtin_cooler() -> LcdTemplate {
    let label_color = [230, 238, 246, 255];
    let value_color = [224, 240, 255, 255];

    LcdTemplate {
        id: BUILTIN_COOLER_ID.to_string(),
        name: "Cooler (default)".to_string(),
        base_width: 480,
        base_height: 480,
        background: TemplateBackground::Builtin {
            asset: BuiltinAsset::CoolerBackground,
        },
        rotated: false,
        target_device: None,
        widgets: vec![
            widget(
                "label-cpu",
                WidgetKind::Label {
                    text: "CPU".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::JetBrainsMono,
                    },
                    font_size: 26.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                170.0,
                232.0,
                100.0,
                28.0,
            ),
            widget(
                "label-temp",
                WidgetKind::Label {
                    text: "TEMP".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::JetBrainsMono,
                    },
                    font_size: 26.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                318.0,
                167.0,
                100.0,
                28.0,
            ),
            widget(
                "label-cores",
                WidgetKind::Label {
                    text: "CPU CORES".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::JetBrainsMono,
                    },
                    font_size: 26.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                240.0,
                334.0,
                200.0,
                28.0,
            ),
            widget(
                "gauge-cpu",
                WidgetKind::Speedometer {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 180.0,
                    sweep_angle: 180.0,
                    needle_color: [224, 240, 255, 255],
                    tick_color: [120, 140, 160, 255],
                    tick_count: 10,
                    background_color: [40, 40, 40, 255],
                    ranges: default_ranges(),
                    show_gauge: false,
                    show_needle: true,
                    needle_width: 14.0,
                    needle_length_pct: 0.95,
                    needle_border_color: [174, 10, 16, 255],
                    needle_border_width: 1.5,
                },
                168.0,
                206.0,
                120.0,
                120.0,
            ),
            widget(
                "bar-temp",
                WidgetKind::VerticalBar {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    background_color: [40, 40, 40, 255],
                    corner_radius: 0.0,
                    ranges: default_ranges(),
                },
                317.0,
                206.0,
                7.0,
                32.0,
            ),
            widget(
                "value-cpu",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuUsage,
                    format: "{:.0}".to_string(),
                    unit: "%".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::JetBrainsMono,
                    },
                    font_size: 39.0,
                    color: value_color,
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: default_ranges(),
                },
                170.0,
                270.0,
                140.0,
                48.0,
            ),
            widget(
                "value-temp",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuUsage,
                    format: "{:.0}".to_string(),
                    unit: "°C".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::JetBrainsMono,
                    },
                    font_size: 39.0,
                    color: value_color,
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: default_ranges(),
                },
                318.0,
                270.0,
                140.0,
                48.0,
            ),
            widget(
                "core-bars",
                WidgetKind::CoreBars {
                    orientation: BarOrientation::Horizontal,
                    background_color: [40, 40, 40, 255],
                    show_labels: true,
                    ranges: default_ranges(),
                },
                242.0,
                367.0,
                256.0,
                47.0,
            ),
        ],
    }
}

fn builtin_doublegauge() -> LcdTemplate {
    let bg_transparent = [0, 0, 0, 0];
    let outer_green = [40, 255, 137, 220];
    let inner_blue = [32, 209, 255, 220];
    let value_outer_green = [40, 255, 137, 255];
    let value_inner_blue = [32, 209, 255, 255];
    let label_color = [230, 238, 246, 255];
    let header_color = [0, 0, 0, 255];

    LcdTemplate {
        id: BUILTIN_DOUBLEGAUGE_ID.to_string(),
        name: "Doublegauge (default)".to_string(),
        base_width: 400,
        base_height: 400,
        background: TemplateBackground::Builtin {
            asset: BuiltinAsset::DoublegaugeBackground,
        },
        rotated: false,
        target_device: None,
        widgets: vec![
            widget(
                "label-header",
                WidgetKind::Label {
                    text: "CPU".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 50.0,
                    color: header_color,
                    align: TextAlign::Center,
                },
                200.0,
                50.0,
                160.0,
                60.0,
            ),
            widget(
                "label-1",
                WidgetKind::Label {
                    text: "USAGE".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 34.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                200.0,
                113.0,
                240.0,
                40.0,
            ),
            widget(
                "label-2",
                WidgetKind::Label {
                    text: "TEMP".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 34.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                200.0,
                227.0,
                240.0,
                40.0,
            ),
            widget(
                "gauge-usage",
                WidgetKind::RadialGauge {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 302.0,
                    sweep_angle: 296.0,
                    inner_radius_pct: 0.89,
                    background_color: bg_transparent,
                    ranges: vec![SensorRange {
                        max: None,
                        color: [outer_green[0], outer_green[1], outer_green[2]],
                        alpha: outer_green[3],
                    }],
                },
                200.0,
                200.0,
                352.0,
                352.0,
            ),
            widget(
                "gauge-temp",
                WidgetKind::RadialGauge {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 302.0,
                    sweep_angle: 296.0,
                    inner_radius_pct: 0.86,
                    background_color: bg_transparent,
                    ranges: vec![SensorRange {
                        max: None,
                        color: [inner_blue[0], inner_blue[1], inner_blue[2]],
                        alpha: inner_blue[3],
                    }],
                },
                200.0,
                200.0,
                290.0,
                290.0,
            ),
            widget(
                "value-usage",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuUsage,
                    format: "{:.0}".to_string(),
                    unit: "%".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 70.0,
                    color: value_outer_green,
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: Vec::new(),
                },
                200.0,
                160.0,
                160.0,
                72.0,
            ),
            widget(
                "value-temp",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuUsage,
                    format: "{:.0}".to_string(),
                    unit: "°C".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 70.0,
                    color: value_inner_blue,
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: Vec::new(),
                },
                200.0,
                274.0,
                160.0,
                72.0,
            ),
        ],
    }
}
