//! Data model for the `MediaType::Custom` template system.

use crate::media::{SensorRange, SensorSourceConfig};
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
    Builtin {
        asset: BuiltinAsset,
    },
}

impl Default for TemplateBackground {
    fn default() -> Self {
        Self::Color {
            rgb: [0, 0, 0, 255],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinAsset {
    CoolerBackground,
    DoublegaugeBackground,
    Thermometer,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FontRef {
    Builtin { font: BuiltinFont },
    File { path: PathBuf },
}

impl Default for FontRef {
    fn default() -> Self {
        Self::Builtin {
            font: BuiltinFont::VictorMono,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinFont {
    VictorMono,
    JetBrainsMono,
    Digital7,
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
        ]
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
