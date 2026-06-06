//! Annular (donut) progress gauge with optional rounded corners on the bg
//! ring and the value fill (independently configurable, in pixels).

use super::super::helpers::{range_color, unit_interval};
use image::{Rgba, RgbaImage};
use lianli_shared::media::SensorRange;
use lianli_shared::template::GradientStop;
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
    gradient: bool,
    gradient_stops: &[GradientStop],
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
    let solid_value_color = range_color(ranges, u);
    let gradient_stops = if gradient {
        prepare_gradient_stops(gradient_stops)
    } else {
        Vec::new()
    };

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
                let value_color = if gradient {
                    let pixel_u = (diff / sweep).clamp(0.0, 1.0);
                    gradient_color(&gradient_stops, pixel_u)
                } else {
                    solid_value_color
                };

                if value_color[3] == 0 {
                    continue;
                }

                sub.put_pixel(x, y, value_color);
            }
        }
    }
}

fn prepare_gradient_stops(stops: &[GradientStop]) -> Vec<(f32, [u8; 4])> {
    let mut out: Vec<(f32, [u8; 4])> = if stops.is_empty() {
        vec![
            (0.0, [45, 110, 255, 255]),
            (0.50, [170, 80, 255, 255]),
            (1.0, [255, 80, 190, 255]),
        ]
    } else {
        stops
            .iter()
            .map(|s| {
                (
                    (s.position / 100.0).clamp(0.0, 1.0),
                    [s.color[0], s.color[1], s.color[2], s.alpha],
                )
            })
            .collect()
    };

    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    out
}

fn gradient_color(stops: &[(f32, [u8; 4])], t: f32) -> Rgba<u8> {
    if stops.is_empty() {
        return Rgba([255, 255, 255, 255]);
    }

    let t = t.clamp(0.0, 1.0);

    if t <= stops[0].0 {
        return Rgba(stops[0].1);
    }

    let last = stops.last().unwrap();
    if t >= last.0 {
        return Rgba(last.1);
    }

    for pair in stops.windows(2) {
        let (p0, c0) = pair[0];
        let (p1, c1) = pair[1];

        if t >= p0 && t <= p1 {
            let local_t = (t - p0) / (p1 - p0).max(f32::EPSILON);

            return Rgba([
                lerp_u8(c0[0], c1[0], local_t),
                lerp_u8(c0[1], c1[1], local_t),
                lerp_u8(c0[2], c1[2], local_t),
                lerp_u8(c0[3], c1[3], local_t),
            ]);
        }
    }

    Rgba(last.1)
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);

    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
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
