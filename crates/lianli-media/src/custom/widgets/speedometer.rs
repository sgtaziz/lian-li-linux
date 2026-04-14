//! Analog speedometer: arc gauge + needle with ticks.

use super::super::helpers::{draw_annulus, range_color, unit_interval};
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_antialiased_line_segment_mut, draw_polygon_mut};
use imageproc::pixelops::interpolate;
use imageproc::point::Point;
use lianli_shared::media::SensorRange;
use std::f32::consts::PI;

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn draw(
    sub: &mut RgbaImage,
    value: f32,
    value_min: f32,
    value_max: f32,
    start_angle: f32,
    sweep_angle: f32,
    needle_color: [u8; 4],
    tick_color: [u8; 4],
    tick_count: u32,
    background_color: [u8; 4],
    ranges: &[SensorRange],
    show_gauge: bool,
    show_needle: bool,
    needle_width_pct: f32,
    needle_length_pct: f32,
    needle_border_color: [u8; 4],
    needle_border_width: f32,
    uniform_scale: f32,
) {
    let (w, h) = (sub.width() as f32, sub.height() as f32);
    let center = (w / 2.0, h / 2.0);
    let r_outer = (w.min(h) / 2.0).max(1.0);
    let r_inner = r_outer * 0.82;
    let u = unit_interval(value, value_min, value_max);

    if show_gauge {
        let bg = Rgba(background_color);
        draw_annulus(sub, center, r_inner, r_outer, start_angle, sweep_angle, bg);

        if !ranges.is_empty() {
            let fill_color = range_color(ranges, u);
            let fill_sweep = sweep_angle * u;
            if fill_sweep.abs() > 0.01 {
                draw_annulus(
                    sub,
                    center,
                    r_inner,
                    r_outer,
                    start_angle,
                    fill_sweep,
                    fill_color,
                );
            }
        }

        let tick = Rgba(tick_color);
        if tick_count > 0 && tick[3] > 0 {
            for i in 0..=tick_count {
                let t = i as f32 / tick_count as f32;
                let angle = (start_angle + sweep_angle * t).to_radians();
                let (sx, sy) = (
                    center.0 + r_inner * angle.cos(),
                    center.1 + r_inner * angle.sin(),
                );
                let (ex, ey) = (
                    center.0 + r_outer * angle.cos(),
                    center.1 + r_outer * angle.sin(),
                );
                draw_antialiased_line_segment_mut(
                    sub,
                    (sx as i32, sy as i32),
                    (ex as i32, ey as i32),
                    tick,
                    interpolate,
                );
            }
        }
    }

    if show_needle {
        let needle_angle = start_angle + sweep_angle * u;
        let needle = Rgba(needle_color);
        let border = Rgba(needle_border_color);
        let start_len = 4.0 * uniform_scale;
        let length = r_inner * needle_length_pct.clamp(0.1, 1.5);
        let width = (needle_width_pct * uniform_scale).max(2.0) as i32;
        draw_needle(
            sub,
            center,
            needle_angle,
            start_len,
            length,
            width,
            needle,
            border,
            needle_border_width,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_needle(
    img: &mut RgbaImage,
    center: (f32, f32),
    angle_deg: f32,
    start_len: f32,
    length: f32,
    width: i32,
    color: Rgba<u8>,
    border_color: Rgba<u8>,
    border_width: f32,
) {
    let angle_rad = angle_deg.to_radians();
    let orth = angle_rad + PI / 2.0;
    let base = Point {
        x: center.0 + angle_rad.cos() * start_len,
        y: center.1 + angle_rad.sin() * start_len,
    };
    let tip = Point {
        x: center.0 + angle_rad.cos() * length,
        y: center.1 + angle_rad.sin() * length,
    };
    let half = (width / 2) as f32;
    let p1 = Point {
        x: base.x + orth.cos() * half,
        y: base.y + orth.sin() * half,
    };
    let p2 = Point {
        x: base.x - orth.cos() * half,
        y: base.y - orth.sin() * half,
    };
    let poly = vec![
        Point::new(p1.x as i32, p1.y as i32),
        Point::new(tip.x as i32, tip.y as i32),
        Point::new(p2.x as i32, p2.y as i32),
    ];
    draw_polygon_mut(img, &poly, color);

    if border_width <= 0.0 {
        return;
    }
    // imageproc's antialiased lines are 1px wide, so we stack offset copies for thickness.
    let layers = (border_width.round() as i32).max(1);
    let edges = [(p1, tip), (tip, p2), (p2, p1)];
    for layer in 0..layers {
        let offset = layer as f32 - (layers as f32 - 1.0) / 2.0;
        for (a, b) in edges.iter() {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            let nx = -dy / len * offset;
            let ny = dx / len * offset;
            draw_antialiased_line_segment_mut(
                img,
                ((a.x + nx) as i32, (a.y + ny) as i32),
                ((b.x + nx) as i32, (b.y + ny) as i32),
                border_color,
                interpolate,
            );
        }
    }
}
