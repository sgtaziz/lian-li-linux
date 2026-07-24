mod to_editor;
mod to_template;

pub(super) use to_editor::{blank_editor_widget, widget_to_editor};
pub(super) use to_template::{
    label_to_font_ref, make_default_widget, parse_align, parse_sensor_source, parse_u8,
};

use crate::EditorWidget;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::Widget;
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
