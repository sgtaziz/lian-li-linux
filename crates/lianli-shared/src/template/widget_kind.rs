use super::{BarOrientation, FontRef, ImageFit, TextAlign};
use crate::media::{SensorRange, SensorSourceConfig};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_alpha() -> u8 {
    255
}

pub fn default_gradient_stops() -> Vec<GradientStop> {
    vec![
        GradientStop {
            position: 0.0,
            color: [45, 110, 255],
            alpha: 255,
        },
        GradientStop {
            position: 50.0,
            color: [170, 80, 255],
            alpha: 255,
        },
        GradientStop {
            position: 100.0,
            color: [255, 80, 190],
            alpha: 255,
        },
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: [u8; 3],
    #[serde(default = "default_alpha")]
    pub alpha: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetKind {
    Label {
        text: String,
        #[serde(default)]
        font: FontRef,
        font_size: f32,
        #[serde(with = "super::rgba_serde")]
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        letter_spacing: f32,
    },
    ValueText {
        source: SensorSourceConfig,
        #[serde(default = "default_value_format")]
        format: String,
        #[serde(default)]
        unit: String,
        #[serde(default)]
        font: FontRef,
        font_size: f32,
        #[serde(with = "super::rgba_serde")]
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
        #[serde(default = "default_value_min")]
        value_min: f32,
        #[serde(default = "default_value_max")]
        value_max: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(default)]
        letter_spacing: f32,
    },
    RadialGauge {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        start_angle: f32,
        sweep_angle: f32,
        #[serde(default = "default_inner_radius_pct")]
        inner_radius_pct: f32,
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(default)]
        bg_corner_radius: f32,
        #[serde(default)]
        value_corner_radius: f32,
        #[serde(default)]
        gradient: bool,
        #[serde(default = "default_gradient_stops")]
        gradient_stops: Vec<GradientStop>,
    },
    VerticalBar {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    HorizontalBar {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    Speedometer {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        start_angle: f32,
        sweep_angle: f32,
        #[serde(with = "super::rgba_serde")]
        needle_color: [u8; 4],
        #[serde(with = "super::rgba_serde")]
        tick_color: [u8; 4],
        #[serde(default = "default_tick_count")]
        tick_count: u32,
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(default = "super::default_true")]
        show_gauge: bool,
        #[serde(default = "super::default_true")]
        show_needle: bool,
        #[serde(default = "default_needle_width")]
        needle_width: f32,
        #[serde(default = "default_needle_length_pct")]
        needle_length_pct: f32,
        #[serde(default = "default_needle_border_color", with = "super::rgba_serde")]
        needle_border_color: [u8; 4],
        #[serde(default = "default_needle_border_width")]
        needle_border_width: f32,
    },
    CoreBars {
        #[serde(default)]
        orientation: BarOrientation,
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default = "super::default_true")]
        show_labels: bool,
        #[serde(default)]
        ranges: Vec<SensorRange>,
    },
    Image {
        path: PathBuf,
        #[serde(default = "default_opacity")]
        opacity: f32,
        #[serde(default)]
        fit: ImageFit,
    },
    Video {
        path: PathBuf,
        #[serde(default = "super::default_true")]
        loop_playback: bool,
        #[serde(default = "default_opacity")]
        opacity: f32,
        #[serde(default)]
        fit: ImageFit,
    },
    Sparkline {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        #[serde(default)]
        auto_range: bool,
        #[serde(default = "default_sparkline_history")]
        history_length: u32,
        #[serde(default = "default_sparkline_line_width")]
        line_width: f32,
        #[serde(with = "super::rgba_serde", default = "default_sparkline_line_color")]
        line_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_sparkline_fill_color")]
        fill_color: [u8; 4],
        #[serde(with = "super::rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(with = "super::rgba_serde", default = "default_sparkline_border_color")]
        border_color: [u8; 4],
        #[serde(default)]
        border_width: f32,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        padding: f32,
        #[serde(default)]
        show_points: bool,
        #[serde(default = "default_sparkline_point_radius")]
        point_radius: f32,
        #[serde(default)]
        show_baseline: bool,
        #[serde(default)]
        baseline_value: f32,
        #[serde(
            with = "super::rgba_serde",
            default = "default_sparkline_baseline_color"
        )]
        baseline_color: [u8; 4],
        #[serde(default = "default_sparkline_baseline_width")]
        baseline_width: f32,
        #[serde(default)]
        smooth: bool,
        #[serde(default)]
        scroll_rtl: bool,
        #[serde(default)]
        fill_from_ranges: bool,
        #[serde(default)]
        range_blend: bool,
        #[serde(default)]
        show_gridlines: bool,
        #[serde(default = "default_sparkline_gridline_h")]
        gridlines_horizontal: u32,
        #[serde(default)]
        gridlines_vertical: u32,
        #[serde(
            with = "super::rgba_serde",
            default = "default_sparkline_gridline_color"
        )]
        gridline_color: [u8; 4],
        #[serde(default = "default_sparkline_gridline_width")]
        gridline_width: f32,
        #[serde(default)]
        show_axis_labels: bool,
        #[serde(default = "default_sparkline_axis_label_count")]
        axis_label_count: u32,
        #[serde(default)]
        axis_labels_on_right: bool,
        #[serde(default = "default_sparkline_axis_label_format")]
        axis_label_format: String,
        #[serde(default)]
        axis_label_font: FontRef,
        #[serde(default = "default_sparkline_axis_label_size")]
        axis_label_size: f32,
        #[serde(
            with = "super::rgba_serde",
            default = "default_sparkline_axis_label_color"
        )]
        axis_label_color: [u8; 4],
        #[serde(default = "default_sparkline_axis_label_padding")]
        axis_label_padding: f32,
    },
    ClockDigital {
        #[serde(default = "default_clock_format")]
        format: String,
        #[serde(default)]
        font: FontRef,
        font_size: f32,
        #[serde(with = "super::rgba_serde")]
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        letter_spacing: f32,
    },
    ClockAnalog {
        #[serde(with = "super::rgba_serde", default = "default_clock_face_color")]
        face_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_tick_color")]
        tick_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_tick_color")]
        minor_tick_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_hand_color")]
        hour_hand_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_hand_color")]
        minute_hand_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_second_color")]
        second_hand_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_hand_color")]
        hub_color: [u8; 4],
        #[serde(with = "super::rgba_serde", default = "default_clock_numbers_color")]
        numbers_color: [u8; 4],
        #[serde(default)]
        numbers_font: FontRef,
        #[serde(default = "default_clock_numbers_size")]
        numbers_font_size: f32,
        #[serde(default = "super::default_true")]
        show_seconds: bool,
        #[serde(default = "super::default_true")]
        show_hour_ticks: bool,
        #[serde(default = "super::default_true")]
        show_minor_ticks: bool,
        #[serde(default)]
        show_numbers: bool,
        #[serde(default = "default_clock_hand_width_hour")]
        hour_hand_width: f32,
        #[serde(default = "default_clock_hand_width_minute")]
        minute_hand_width: f32,
        #[serde(default = "default_clock_hand_width_second")]
        second_hand_width: f32,
        #[serde(default = "default_clock_hand_length_hour")]
        hour_hand_length_pct: f32,
        #[serde(default = "default_clock_hand_length_minute")]
        minute_hand_length_pct: f32,
        #[serde(default = "default_clock_hand_length_second")]
        second_hand_length_pct: f32,
        #[serde(default = "default_clock_tick_length_hour")]
        hour_tick_length_pct: f32,
        #[serde(default = "default_clock_tick_length_minor")]
        minor_tick_length_pct: f32,
        #[serde(default = "default_clock_tick_width_hour")]
        hour_tick_width: f32,
        #[serde(default = "default_clock_tick_width_minor")]
        minor_tick_width: f32,
        #[serde(default = "default_clock_hub_radius")]
        hub_radius: f32,
    },
}

fn default_value_format() -> String {
    "{:.0}".to_string()
}

fn default_value_min() -> f32 {
    0.0
}

fn default_value_max() -> f32 {
    100.0
}

fn default_inner_radius_pct() -> f32 {
    0.78
}

fn default_tick_count() -> u32 {
    10
}

fn default_needle_width() -> f32 {
    14.0
}

fn default_needle_length_pct() -> f32 {
    0.95
}

fn default_needle_border_color() -> [u8; 4] {
    [174, 10, 16, 255]
}

fn default_needle_border_width() -> f32 {
    1.5
}

fn default_opacity() -> f32 {
    1.0
}

fn default_sparkline_history() -> u32 {
    60
}

fn default_sparkline_line_width() -> f32 {
    2.0
}

fn default_sparkline_line_color() -> [u8; 4] {
    [80, 180, 240, 255]
}

fn default_sparkline_fill_color() -> [u8; 4] {
    [80, 180, 240, 80]
}

fn default_sparkline_border_color() -> [u8; 4] {
    [80, 90, 110, 255]
}

fn default_sparkline_baseline_color() -> [u8; 4] {
    [140, 140, 160, 160]
}

fn default_sparkline_baseline_width() -> f32 {
    1.0
}

fn default_sparkline_point_radius() -> f32 {
    2.5
}

fn default_sparkline_gridline_h() -> u32 {
    3
}

fn default_sparkline_gridline_color() -> [u8; 4] {
    [120, 120, 140, 90]
}

fn default_sparkline_gridline_width() -> f32 {
    1.0
}

fn default_sparkline_axis_label_count() -> u32 {
    3
}

fn default_sparkline_axis_label_format() -> String {
    "{:.0}".to_string()
}

fn default_sparkline_axis_label_size() -> f32 {
    11.0
}

fn default_sparkline_axis_label_color() -> [u8; 4] {
    [200, 200, 210, 220]
}

fn default_sparkline_axis_label_padding() -> f32 {
    4.0
}

fn default_clock_format() -> String {
    "%H:%M".to_string()
}

fn default_clock_face_color() -> [u8; 4] {
    [30, 30, 30, 255]
}

fn default_clock_tick_color() -> [u8; 4] {
    [220, 220, 220, 255]
}

fn default_clock_hand_color() -> [u8; 4] {
    [240, 240, 240, 255]
}

fn default_clock_second_color() -> [u8; 4] {
    [220, 40, 40, 255]
}

fn default_clock_numbers_color() -> [u8; 4] {
    [230, 230, 230, 255]
}

fn default_clock_numbers_size() -> f32 {
    24.0
}

fn default_clock_hand_width_hour() -> f32 {
    6.0
}

fn default_clock_hand_width_minute() -> f32 {
    4.0
}

fn default_clock_hand_width_second() -> f32 {
    2.0
}

fn default_clock_hand_length_hour() -> f32 {
    0.55
}

fn default_clock_hand_length_minute() -> f32 {
    0.8
}

fn default_clock_hand_length_second() -> f32 {
    0.9
}

fn default_clock_tick_length_hour() -> f32 {
    0.12
}

fn default_clock_tick_length_minor() -> f32 {
    0.05
}

fn default_clock_tick_width_hour() -> f32 {
    3.0
}

fn default_clock_tick_width_minor() -> f32 {
    1.5
}

fn default_clock_hub_radius() -> f32 {
    6.0
}

impl WidgetKind {
    pub fn kind_id(&self) -> &'static str {
        match self {
            Self::Label { .. } => "label",
            Self::ValueText { .. } => "value_text",
            Self::RadialGauge { .. } => "radial_gauge",
            Self::VerticalBar { .. } => "vertical_bar",
            Self::HorizontalBar { .. } => "horizontal_bar",
            Self::Speedometer { .. } => "speedometer",
            Self::CoreBars { .. } => "core_bars",
            Self::Image { .. } => "image",
            Self::Video { .. } => "video",
            Self::ClockDigital { .. } => "clock_digital",
            Self::ClockAnalog { .. } => "clock_analog",
            Self::Sparkline { .. } => "sparkline",
        }
    }

    pub fn friendly_name(&self) -> &'static str {
        Self::friendly_name_for(self.kind_id())
    }

    pub fn friendly_name_for(kind_id: &str) -> &'static str {
        match kind_id {
            "label" => "Label",
            "value_text" => "Sensor Value",
            "radial_gauge" => "Radial Gauge",
            "vertical_bar" => "Vertical Bar",
            "horizontal_bar" => "Horizontal Bar",
            "speedometer" => "Speedometer",
            "core_bars" => "Core Usage",
            "image" => "Image",
            "video" => "Video",
            "clock_digital" => "Clock (Digital)",
            "clock_analog" => "Clock (Analog)",
            "sparkline" => "Sparkline",
            _ => "Widget",
        }
    }

    pub fn kind_id_for_friendly(label: &str) -> Option<&'static str> {
        Self::all_kind_ids()
            .iter()
            .copied()
            .find(|id| Self::friendly_name_for(id) == label)
    }

    pub fn all_kind_ids() -> &'static [&'static str] {
        &[
            "label",
            "value_text",
            "radial_gauge",
            "vertical_bar",
            "horizontal_bar",
            "speedometer",
            "core_bars",
            "image",
            "video",
            "clock_digital",
            "clock_analog",
            "sparkline",
        ]
    }

    pub fn source_config_mut(&mut self) -> Option<&mut SensorSourceConfig> {
        match self {
            Self::ValueText { source, .. }
            | Self::RadialGauge { source, .. }
            | Self::VerticalBar { source, .. }
            | Self::HorizontalBar { source, .. }
            | Self::Speedometer { source, .. }
            | Self::Sparkline { source, .. } => Some(source),
            _ => None,
        }
    }
}
