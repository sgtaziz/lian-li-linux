//! Data model for the `MediaType::Custom` template system.

use crate::media::{SensorRange, SensorSourceConfig};
use crate::sensors::{pick_source_for_category, SensorCategory, SensorInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Accepts both `[r,g,b]` (alpha defaults to 255) and `[r,g,b,a]` so older
/// hand-written templates keep loading after the alpha channel was added.
pub mod rgba_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(c: &[u8; 4], s: S) -> Result<S::Ok, S::Error> {
        c.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 4], D::Error> {
        let v: Vec<u8> = Vec::deserialize(d)?;
        match v.len() {
            3 => Ok([v[0], v[1], v[2], 255]),
            4 => Ok([v[0], v[1], v[2], v[3]]),
            n => Err(serde::de::Error::custom(format!(
                "expected 3 or 4 color components, got {n}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LcdTemplate {
    pub id: String,
    pub name: String,
    pub base_width: u32,
    pub base_height: u32,
    pub background: TemplateBackground,
    #[serde(default)]
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub rotated: bool,
    #[serde(default)]
    pub target_device: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TemplateBackground {
    Color {
        #[serde(with = "rgba_serde")]
        rgb: [u8; 4],
    },
    Image {
        path: PathBuf,
    },
}

impl Default for TemplateBackground {
    fn default() -> Self {
        Self::Color {
            rgb: [0, 0, 0, 255],
        }
    }
}

/// `x`/`y` are the widget center; `width`/`height` are pre-rotation bounds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Widget {
    pub id: String,
    pub kind: WidgetKind,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub update_interval_ms: Option<u64>,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_category: Option<SensorCategory>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetKind {
    Label {
        text: String,
        #[serde(default)]
        font: FontRef,
        font_size: f32,
        #[serde(with = "rgba_serde")]
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
        #[serde(with = "rgba_serde")]
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
        #[serde(with = "rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(default)]
        bg_corner_radius: f32,
        #[serde(default)]
        value_corner_radius: f32,
    },
    VerticalBar {
        source: SensorSourceConfig,
        value_min: f32,
        value_max: f32,
        #[serde(with = "rgba_serde")]
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
        #[serde(with = "rgba_serde")]
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
        #[serde(with = "rgba_serde")]
        needle_color: [u8; 4],
        #[serde(with = "rgba_serde")]
        tick_color: [u8; 4],
        #[serde(default = "default_tick_count")]
        tick_count: u32,
        #[serde(with = "rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(default = "default_true")]
        show_gauge: bool,
        #[serde(default = "default_true")]
        show_needle: bool,
        #[serde(default = "default_needle_width")]
        needle_width: f32,
        #[serde(default = "default_needle_length_pct")]
        needle_length_pct: f32,
        #[serde(default = "default_needle_border_color", with = "rgba_serde")]
        needle_border_color: [u8; 4],
        #[serde(default = "default_needle_border_width")]
        needle_border_width: f32,
    },
    CoreBars {
        #[serde(default)]
        orientation: BarOrientation,
        #[serde(with = "rgba_serde")]
        background_color: [u8; 4],
        #[serde(default = "default_true")]
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
        #[serde(default = "default_true")]
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
        #[serde(with = "rgba_serde", default = "default_sparkline_line_color")]
        line_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_sparkline_fill_color")]
        fill_color: [u8; 4],
        #[serde(with = "rgba_serde")]
        background_color: [u8; 4],
        #[serde(default)]
        ranges: Vec<SensorRange>,
        #[serde(with = "rgba_serde", default = "default_sparkline_border_color")]
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
        #[serde(with = "rgba_serde", default = "default_sparkline_baseline_color")]
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
        #[serde(with = "rgba_serde", default = "default_sparkline_gridline_color")]
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
        #[serde(with = "rgba_serde", default = "default_sparkline_axis_label_color")]
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
        #[serde(with = "rgba_serde")]
        color: [u8; 4],
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        letter_spacing: f32,
    },
    ClockAnalog {
        #[serde(with = "rgba_serde", default = "default_clock_face_color")]
        face_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_tick_color")]
        tick_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_tick_color")]
        minor_tick_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_hand_color")]
        hour_hand_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_hand_color")]
        minute_hand_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_second_color")]
        second_hand_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_hand_color")]
        hub_color: [u8; 4],
        #[serde(with = "rgba_serde", default = "default_clock_numbers_color")]
        numbers_color: [u8; 4],
        #[serde(default)]
        numbers_font: FontRef,
        #[serde(default = "default_clock_numbers_size")]
        numbers_font_size: f32,
        #[serde(default = "default_true")]
        show_seconds: bool,
        #[serde(default = "default_true")]
        show_hour_ticks: bool,
        #[serde(default = "default_true")]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFit {
    #[default]
    Stretch,
    Contain,
    Cover,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontRef {
    #[serde(default)]
    pub path: Option<PathBuf>,
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

pub fn resolve_sensor_categories(template: &mut LcdTemplate, sensors: &[SensorInfo]) {
    for widget in template.widgets.iter_mut() {
        let Some(category) = widget.sensor_category.take() else {
            continue;
        };
        let Some(source_ref) = widget.kind.source_config_mut() else {
            continue;
        };
        if let Some(new_source) = pick_source_for_category(category, sensors) {
            *source_ref = new_source;
        }
    }
}

impl LcdTemplate {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("template id must not be empty".into());
        }
        if self.name.trim().is_empty() {
            return Err(format!("template '{}' name must not be empty", self.id));
        }
        if self.base_width == 0 || self.base_height == 0 {
            return Err(format!(
                "template '{}' base dimensions must be positive",
                self.id
            ));
        }
        for (i, w) in self.widgets.iter().enumerate() {
            if w.width <= 0.0 || w.height <= 0.0 {
                return Err(format!(
                    "template '{}' widget[{i}] '{}' has non-positive size",
                    self.id, w.id
                ));
            }
            if let Some(ms) = w.update_interval_ms {
                if !(100..=10_000).contains(&ms) {
                    return Err(format!(
                        "template '{}' widget[{i}] update_interval_ms must be in [100, 10000]",
                        self.id
                    ));
                }
            }
        }
        Ok(())
    }
}
