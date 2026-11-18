//! Data model for the `MediaType::Custom` template system.

use crate::media::SensorSourceConfig;
use crate::sensors::{pick_source_for_category, SensorCategory, SensorInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub mod catalog;
mod catalog_recovery;
mod catalog_review;
mod catalog_storage;
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
    #[serde(default, deserialize_with = "crate::serde_limits::widgets")]
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
    Video {
        path: PathBuf,
        #[serde(default)]
        fps: Option<f32>,
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
    /// While set, `kind`'s source is a portable placeholder that is not bound
    /// to a sensor on this machine, so the widget renders as unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_category: Option<SensorCategory>,
}

impl Widget {
    pub fn bound_sensor_source(&self) -> Option<&SensorSourceConfig> {
        self.kind
            .source_config()
            .filter(|_| self.sensor_category.is_none())
    }
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

/// The user's sensor choice per category; `None` leaves the category unbound.
pub type SensorBindings = BTreeMap<SensorCategory, Option<SensorSourceConfig>>;

/// Binds each placeholder widget to the chosen sensor, or to the detected one
/// when no choice was made. A category without a sensor stays unbound.
pub fn resolve_sensor_categories(
    template: &mut LcdTemplate,
    bindings: &SensorBindings,
    sensors: &[SensorInfo],
) {
    for widget in template.widgets.iter_mut() {
        let Some(category) = widget.sensor_category else {
            continue;
        };
        let Some(source) = widget.kind.source_config_mut() else {
            continue;
        };
        let chosen = match bindings.get(&category) {
            Some(choice) => choice.clone(),
            None => pick_source_for_category(category, sensors),
        };
        if let Some(chosen) = chosen {
            *source = chosen;
            widget.sensor_category = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::{NvidiaMetric, SensorSource, Unit};

    const CATEGORIES: [&str; 4] = ["cpu_temp", "gpu_temp", "gpu_usage", "cpu_usage"];

    fn portable_template() -> LcdTemplate {
        let widgets: Vec<_> = CATEGORIES
            .iter()
            .map(|category| {
                serde_json::json!({
                    "id": category, "x": 0, "y": 0, "width": 10, "height": 10,
                    "sensor_category": category,
                    "kind": {"type": "value_text", "source": {"type": "cpu_usage"},
                        "font_size": 12, "color": [255, 255, 255]}
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "id": "portable", "name": "Portable", "base_width": 100, "base_height": 100,
            "background": {"type": "color", "rgb": [0, 0, 0]},
            "widgets": widgets
        }))
        .unwrap()
    }

    fn sensor(source: SensorSource, unit: Unit) -> SensorInfo {
        SensorInfo {
            source,
            sensor_name: None,
            display_name: None,
            divider: 1,
            unit,
            current_value: None,
        }
    }

    fn detected_sensors() -> Vec<SensorInfo> {
        vec![
            sensor(
                SensorSource::Hwmon {
                    name: "k10temp".into(),
                    label: "Tctl".into(),
                    device_path: String::new(),
                },
                Unit::C,
            ),
            sensor(
                SensorSource::NvidiaGpu {
                    gpu_index: 0,
                    metric: NvidiaMetric::Temp,
                },
                Unit::C,
            ),
            sensor(
                SensorSource::NvidiaGpu {
                    gpu_index: 0,
                    metric: NvidiaMetric::Usage,
                },
                Unit::PERCENT,
            ),
        ]
    }

    fn widget<'a>(template: &'a LcdTemplate, id: &str) -> &'a Widget {
        template.widgets.iter().find(|w| w.id == id).unwrap()
    }

    #[test]
    fn missing_sensors_leave_placeholders_unbound() {
        let mut template = portable_template();
        resolve_sensor_categories(&mut template, &SensorBindings::new(), &[]);
        for id in ["cpu_temp", "gpu_temp", "gpu_usage"] {
            let unbound = widget(&template, id);
            assert!(unbound.sensor_category.is_some(), "{id} lost its category");
            assert!(
                unbound.bound_sensor_source().is_none(),
                "{id} reads its placeholder"
            );
        }
        let cpu = widget(&template, "cpu_usage");
        assert_eq!(cpu.sensor_category, None);
        assert!(matches!(
            cpu.bound_sensor_source(),
            Some(SensorSourceConfig::CpuUsage)
        ));
    }

    #[test]
    fn detected_sensors_bind_every_category() {
        let mut template = portable_template();
        resolve_sensor_categories(&mut template, &SensorBindings::new(), &detected_sensors());
        assert!(template.widgets.iter().all(|w| w.sensor_category.is_none()));
        assert!(matches!(
            widget(&template, "gpu_usage").bound_sensor_source(),
            Some(SensorSourceConfig::NvidiaGpu {
                metric: NvidiaMetric::Usage,
                ..
            })
        ));
        assert!(matches!(
            widget(&template, "cpu_temp").bound_sensor_source(),
            Some(SensorSourceConfig::Hwmon { name, .. }) if name == "k10temp"
        ));
    }

    #[test]
    fn user_choices_override_detection_and_can_leave_a_category_unbound() {
        let mut template = portable_template();
        let bindings: SensorBindings = serde_json::from_value(serde_json::json!({
            "cpu_temp": null,
            "gpu_temp": {"type": "hwmon", "name": "amdgpu", "label": "edge"}
        }))
        .unwrap();
        resolve_sensor_categories(&mut template, &bindings, &detected_sensors());
        let cpu_temp = widget(&template, "cpu_temp");
        assert_eq!(cpu_temp.sensor_category, Some(SensorCategory::CpuTemp));
        assert!(cpu_temp.bound_sensor_source().is_none());
        assert!(matches!(
            widget(&template, "gpu_temp").bound_sensor_source(),
            Some(SensorSourceConfig::Hwmon { name, .. }) if name == "amdgpu"
        ));
        assert!(matches!(
            widget(&template, "gpu_usage").bound_sensor_source(),
            Some(SensorSourceConfig::NvidiaGpu { .. })
        ));
    }

    #[test]
    fn video_background_defaults_to_the_template_frame_rate() {
        let background: TemplateBackground =
            serde_json::from_value(serde_json::json!({"type": "video", "path": "loop.mp4"}))
                .unwrap();
        assert!(matches!(
            background,
            TemplateBackground::Video { path, fps: None } if path == std::path::Path::new("loop.mp4")
        ));
    }
}
