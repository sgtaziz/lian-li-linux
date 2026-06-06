//! Data model for the `MediaType::Custom` template system.

use crate::sensors::{pick_source_for_category, SensorCategory, SensorInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod widget_kind;
pub use widget_kind::{default_gradient_stops, GradientStop, WidgetKind};

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
