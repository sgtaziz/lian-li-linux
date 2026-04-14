//! Vertical/horizontal fill bar with optional rounded corners.

use super::super::helpers::{
    fill_rect_clipped_rounded, fill_rounded_rect, range_color, unit_interval,
};
use image::{Rgba, RgbaImage};
use lianli_shared::media::SensorRange;

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn draw(
    sub: &mut RgbaImage,
    value: f32,
    value_min: f32,
    value_max: f32,
    background_color: [u8; 4],
    corner_radius: f32,
    ranges: &[SensorRange],
    is_vertical: bool,
) {
    let (w, h) = (sub.width(), sub.height());
    let bg = Rgba(background_color);
    fill_rounded_rect(sub, 0, 0, w, h, corner_radius, bg);

    let u = unit_interval(value, value_min, value_max);
    let color = range_color(ranges, u);
    if u <= 0.0 || color[3] == 0 {
        return;
    }
    let (fx, fy, fw, fh) = if is_vertical {
        let fill_h = ((h as f32) * u).round() as u32;
        if fill_h == 0 {
            return;
        }
        (0i32, (h - fill_h) as i32, w, fill_h)
    } else {
        let fill_w = ((w as f32) * u).round() as u32;
        if fill_w == 0 {
            return;
        }
        (0i32, 0i32, fill_w, h)
    };
    fill_rect_clipped_rounded(sub, fx, fy, fw, fh, 0, 0, w, h, corner_radius, color);
}
