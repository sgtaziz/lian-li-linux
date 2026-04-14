//! Per-CPU-core usage bars.

use super::super::helpers::range_color;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use lianli_shared::media::SensorRange;
use lianli_shared::systeminfo::SysSensor;
use lianli_shared::template::BarOrientation;
use rusttype::{Font, Scale};

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn draw(
    sub: &mut RgbaImage,
    orientation: BarOrientation,
    background_color: [u8; 4],
    show_labels: bool,
    ranges: &[SensorRange],
    uniform_scale: f32,
    font_digital7: &Font<'static>,
) {
    let (w, h) = (sub.width(), sub.height());
    let bg = Rgba(background_color);
    if bg[3] > 0 {
        draw_filled_rect_mut(sub, Rect::at(0, 0).of_size(w, h), bg);
    }

    let usage = SysSensor::get_core_usage();
    let num_cores = usage.len().max(1);
    let label_color = Rgba([230, 238, 246, 255]);

    let cell_gap = 2.0_f32;

    match orientation {
        BarOrientation::Horizontal => {
            let cell_w = w as f32 / num_cores as f32;
            for (i, &u10k) in usage.iter().enumerate() {
                let u = (u10k.min(10_000) as f32) / 10_000.0;
                let color = range_color(ranges, u);
                let x_start = (i as f32 * cell_w).round() as i32;
                let x_end = ((i + 1) as f32 * cell_w).round() as i32;
                let bar_w = ((x_end - x_start) as f32 - cell_gap).max(1.0) as u32;
                let bar_h = ((h as f32) * u).round() as u32;
                if bar_h > 0 {
                    draw_filled_rect_mut(
                        sub,
                        Rect::at(x_start, (h - bar_h) as i32).of_size(bar_w, bar_h),
                        color,
                    );
                }
                if show_labels {
                    let label = format!("{}", (i + 1) % 10);
                    draw_text_mut(
                        sub,
                        label_color,
                        x_start + 2,
                        (h as i32) - (10.0 * uniform_scale) as i32,
                        Scale::uniform(9.0 * uniform_scale),
                        font_digital7,
                        &label,
                    );
                }
            }
        }
        BarOrientation::Vertical => {
            let cell_h = h as f32 / num_cores as f32;
            for (i, &u10k) in usage.iter().enumerate() {
                let u = (u10k.min(10_000) as f32) / 10_000.0;
                let color = range_color(ranges, u);
                let y_start = (i as f32 * cell_h).round() as i32;
                let y_end = ((i + 1) as f32 * cell_h).round() as i32;
                let bar_h = ((y_end - y_start) as f32 - cell_gap).max(1.0) as u32;
                let bar_w = ((w as f32) * u).round() as u32;
                if bar_w > 0 {
                    draw_filled_rect_mut(sub, Rect::at(0, y_start).of_size(bar_w, bar_h), color);
                }
                if show_labels {
                    let label = format!("{}", (i + 1) % 10);
                    draw_text_mut(
                        sub,
                        label_color,
                        2,
                        y_start,
                        Scale::uniform(9.0 * uniform_scale),
                        font_digital7,
                        &label,
                    );
                }
            }
        }
    }
}
