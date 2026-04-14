//! Annular (donut) progress gauge with optional rounded corners on the bg
//! ring and the value fill (independently configurable, in pixels).

use super::super::helpers::{range_color, unit_interval};
use image::{Rgba, RgbaImage};
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
    inner_radius_pct: f32,
    background_color: [u8; 4],
    ranges: &[SensorRange],
    bg_corner_radius: f32,
    value_corner_radius: f32,
) {
    let (w, h) = (sub.width(), sub.height());
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let r_outer = (w.min(h) as f32 / 2.0).max(1.0);
    let r_inner = (r_outer * inner_radius_pct.clamp(0.0, 0.99)).max(1.0);
    let half_thickness = (r_outer - r_inner) / 2.0;
    let r_mid = (r_inner + r_outer) / 2.0;

    let sweep = sweep_angle.clamp(0.0, 360.0);
    if sweep <= 0.01 {
        return;
    }
    let u = unit_interval(value, value_min, value_max);
    let fill_sweep = sweep * u;

    let bg = Rgba(background_color);
    let value_color = range_color(ranges, u);

    let bg_cr = clamp_corner(bg_corner_radius, half_thickness, sweep, r_mid);
    let value_cr = clamp_corner(value_corner_radius, half_thickness, fill_sweep, r_mid);

    let start_rad = start_angle.to_radians();
    let sweep_rad = sweep.to_radians();

    let xmin = (cx - r_outer).floor().max(0.0) as u32;
    let xmax = ((cx + r_outer).ceil() as u32).min(w);
    let ymin = (cy - r_outer).floor().max(0.0) as u32;
    let ymax = ((cy + r_outer).ceil() as u32).min(h);

    for y in ymin..ymax {
        for x in xmin..xmax {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d_sq = dx * dx + dy * dy;
            if d_sq < r_inner * r_inner || d_sq > r_outer * r_outer {
                continue;
            }
            let dist = d_sq.sqrt();

            let mut theta = dy.atan2(dx) - start_rad;
            while theta < 0.0 {
                theta += 2.0 * PI;
            }
            while theta >= 2.0 * PI {
                theta -= 2.0 * PI;
            }
            if theta > sweep_rad {
                continue;
            }

            let diff = theta.to_degrees();
            let in_fill = fill_sweep > 0.01 && diff <= fill_sweep;

            let fallthrough_to_bg = in_fill
                && value_cr > 0.0
                && corner_carved(diff, fill_sweep, value_cr, dist, r_mid, half_thickness);

            let is_bg_pixel = !in_fill || fallthrough_to_bg;

            if is_bg_pixel {
                if bg[3] == 0 {
                    continue;
                }
                if bg_cr > 0.0 && corner_carved(diff, sweep, bg_cr, dist, r_mid, half_thickness) {
                    continue;
                }
                sub.put_pixel(x, y, bg);
            } else {
                if value_color[3] == 0 {
                    continue;
                }
                sub.put_pixel(x, y, value_color);
            }
        }
    }
}

fn clamp_corner(raw: f32, half_thickness: f32, arc_sweep_deg: f32, r_mid: f32) -> f32 {
    let arc_len = arc_sweep_deg.to_radians() * r_mid;
    raw.max(0.0).min(half_thickness).min(arc_len / 2.0)
}

fn corner_carved(
    diff_from_start_deg: f32,
    arc_sweep_deg: f32,
    corner_r: f32,
    dist: f32,
    r_mid: f32,
    half_thickness: f32,
) -> bool {
    let rad = PI / 180.0;
    let arc_from_start = diff_from_start_deg * rad * r_mid;
    let arc_from_end = (arc_sweep_deg - diff_from_start_deg) * rad * r_mid;
    let near_start = arc_from_start < corner_r;
    let near_end = arc_from_end < corner_r;
    if !near_start && !near_end {
        return false;
    }
    let offset = dist - r_mid;
    if offset.abs() <= half_thickness - corner_r {
        return false;
    }
    let arc_dist = if near_start {
        arc_from_start
    } else {
        arc_from_end
    };
    let x_from = corner_r - arc_dist;
    let y_from = if offset > 0.0 {
        offset - (half_thickness - corner_r)
    } else {
        offset + (half_thickness - corner_r)
    };
    let corner_dist = (x_from * x_from + y_from * y_from).sqrt();
    corner_dist > corner_r
}
