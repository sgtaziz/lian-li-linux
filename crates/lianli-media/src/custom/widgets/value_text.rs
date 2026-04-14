//! Sensor-driven text display. Color can be threshold-driven via `ranges`.

use super::super::helpers::{draw_text_widget, range_color, unit_interval};
use super::WidgetState;
use image::RgbaImage;
use lianli_shared::media::SensorRange;
use lianli_shared::sensors::read_sensor_value;
use lianli_shared::template::TextAlign;
use rusttype::Font;

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn draw(
    sub: &mut RgbaImage,
    state: &WidgetState,
    font: &Font<'static>,
    size: f32,
    color: [u8; 4],
    align: TextAlign,
    value_min: f32,
    value_max: f32,
    ranges: &[SensorRange],
    ww: u32,
    wh: u32,
    letter_spacing: f32,
) {
    let text = state.last_render_text.clone().unwrap_or_default();
    if text.is_empty() {
        return;
    }
    let resolved_color = if ranges.is_empty() {
        color
    } else {
        let raw = state
            .resolved_sensor
            .as_ref()
            .and_then(|s| read_sensor_value(s).ok())
            .unwrap_or(0.0);
        let u = unit_interval(raw, value_min, value_max);
        range_color(ranges, u).0
    };
    draw_text_widget(
        sub,
        &text,
        font,
        size,
        resolved_color,
        align,
        ww,
        wh,
        letter_spacing,
    );
}
