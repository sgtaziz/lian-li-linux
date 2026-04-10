//! Built-in `LcdTemplate`s shipped with the software.
//!
//! Built-ins live here (not in `lcd_templates.json`) so they're always fresh,
//! upgradable with software updates, and safe from accidental corruption.
//! Editing a built-in in the GUI triggers a "Duplicate to edit" flow that
//! clones the template into the user file under a new id.
//!
//! The ids `cooler-default` and `doublegauge-default` approximate the
//! existing `MediaType::Cooler` and `MediaType::Doublegauge` renderers so
//! users can migrate between the two with minimal visual difference. Exact
//! pixel parity is not guaranteed — the legacy renderers bake custom
//! thermometers and per-core separators that the declarative widget system
//! approximates rather than reproduces verbatim.

use crate::media::{SensorRange, SensorSourceConfig};
use crate::template::{
    BarOrientation, BuiltinFont, FontRef, LcdTemplate, TemplateBackground, TemplateOrientation,
    TextAlign, Widget, WidgetKind,
};
use std::path::PathBuf;

pub const BUILTIN_COOLER_ID: &str = "cooler-default";
pub const BUILTIN_DOUBLEGAUGE_ID: &str = "doublegauge-default";

/// Reserved ids that cannot be deleted or overwritten by user templates.
pub fn is_builtin_id(id: &str) -> bool {
    id == BUILTIN_COOLER_ID || id == BUILTIN_DOUBLEGAUGE_ID
}

/// Return all built-in templates. Called by the template store to merge with
/// user templates during resolution.
pub fn builtin_templates() -> Vec<LcdTemplate> {
    vec![builtin_cooler(), builtin_doublegauge()]
}

/// Look up a built-in template by id. Returns `None` for non-reserved ids.
pub fn builtin_template(id: &str) -> Option<LcdTemplate> {
    match id {
        BUILTIN_COOLER_ID => Some(builtin_cooler()),
        BUILTIN_DOUBLEGAUGE_ID => Some(builtin_doublegauge()),
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
        },
        SensorRange {
            max: Some(80.0),
            color: [220, 140, 0],
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
        },
    ]
}

fn builtin_cooler() -> LcdTemplate {
    // 480×480 base, matching the legacy Cooler renderer.
    let label_color = [230, 238, 246];
    let value_color = [224, 240, 255];
    let bg_gray = [40, 40, 40];

    LcdTemplate {
        id: BUILTIN_COOLER_ID.to_string(),
        name: "Cooler (default)".to_string(),
        base_width: 480,
        base_height: 480,
        background: TemplateBackground::Color { rgb: [10, 14, 22] },
        orientation: TemplateOrientation::Portrait,
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
                234.0,
                120.0,
                32.0,
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
                168.0,
                120.0,
                32.0,
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
                336.0,
                200.0,
                32.0,
            ),
            // Left speedometer-style CPU usage gauge.
            widget(
                "gauge-cpu",
                WidgetKind::Speedometer {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 180.0,
                    sweep_angle: 180.0,
                    needle_color: [224, 240, 255],
                    tick_color: [120, 140, 160],
                    tick_count: 10,
                    background_color: bg_gray,
                },
                168.0,
                190.0,
                140.0,
                140.0,
            ),
            // Right temperature vertical bar (thermometer replacement).
            widget(
                "bar-temp",
                WidgetKind::VerticalBar {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    background_color: bg_gray,
                    corner_radius: 4.0,
                    ranges: default_ranges(),
                },
                318.0,
                210.0,
                24.0,
                80.0,
            ),
            // Live CPU usage %.
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
                    align: TextAlign::Right,
                },
                170.0,
                277.0,
                120.0,
                48.0,
            ),
            // Live temperature value.
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
                    align: TextAlign::Right,
                },
                318.0,
                277.0,
                120.0,
                48.0,
            ),
            // Per-core CPU usage strip along the bottom.
            widget(
                "core-bars",
                WidgetKind::CoreBars {
                    orientation: BarOrientation::Horizontal,
                    color_cold: [0, 200, 0],
                    color_hot: [220, 0, 0],
                    background_color: [30, 30, 30],
                    show_labels: true,
                },
                240.0,
                400.0,
                280.0,
                60.0,
            ),
        ],
    }
}

fn builtin_doublegauge() -> LcdTemplate {
    // 400×400 base, matching the legacy Doublegauge renderer.
    let label_color = [230, 238, 246];
    let bg_gray = [40, 40, 40];
    let outer_green = [40, 255, 137];
    let inner_blue = [32, 209, 255];

    LcdTemplate {
        id: BUILTIN_DOUBLEGAUGE_ID.to_string(),
        name: "Doublegauge (default)".to_string(),
        base_width: 400,
        base_height: 400,
        background: TemplateBackground::Color { rgb: [10, 14, 22] },
        orientation: TemplateOrientation::Portrait,
        widgets: vec![
            widget(
                "header",
                WidgetKind::Label {
                    text: "SYSTEM".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 34.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                200.0,
                50.0,
                320.0,
                40.0,
            ),
            widget(
                "label-outer",
                WidgetKind::Label {
                    text: "CPU".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 34.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                200.0,
                106.0,
                320.0,
                40.0,
            ),
            // Outer gauge (CPU usage).
            widget(
                "gauge-outer",
                WidgetKind::RadialGauge {
                    source: SensorSourceConfig::CpuUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 122.0,
                    sweep_angle: 296.0,
                    inner_radius_pct: 0.88,
                    background_color: bg_gray,
                    ranges: vec![SensorRange {
                        max: None,
                        color: outer_green,
                    }],
                },
                200.0,
                200.0,
                360.0,
                360.0,
            ),
            widget(
                "label-inner",
                WidgetKind::Label {
                    text: "MEM".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 34.0,
                    color: label_color,
                    align: TextAlign::Center,
                },
                200.0,
                220.0,
                320.0,
                40.0,
            ),
            // Inner gauge (memory usage).
            widget(
                "gauge-inner",
                WidgetKind::RadialGauge {
                    source: SensorSourceConfig::MemUsage,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 122.0,
                    sweep_angle: 296.0,
                    inner_radius_pct: 0.85,
                    background_color: bg_gray,
                    ranges: vec![SensorRange {
                        max: None,
                        color: inner_blue,
                    }],
                },
                200.0,
                200.0,
                280.0,
                280.0,
            ),
            // Outer value text.
            widget(
                "value-outer",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuUsage,
                    format: "{:.0}".to_string(),
                    unit: "%".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 70.0,
                    color: outer_green,
                    align: TextAlign::Center,
                },
                200.0,
                156.0,
                200.0,
                80.0,
            ),
            // Inner value text.
            widget(
                "value-inner",
                WidgetKind::ValueText {
                    source: SensorSourceConfig::MemUsage,
                    format: "{:.0}".to_string(),
                    unit: "%".to_string(),
                    font: FontRef::Builtin {
                        font: BuiltinFont::VictorMono,
                    },
                    font_size: 70.0,
                    color: inner_blue,
                    align: TextAlign::Center,
                },
                200.0,
                248.0,
                200.0,
                80.0,
            ),
        ],
    }
}

// Unused imports placeholder — `PathBuf` is imported for future image-background
// defaults. Silence the dead import until then.
#[allow(dead_code)]
fn _path_placeholder(_: PathBuf) {}
