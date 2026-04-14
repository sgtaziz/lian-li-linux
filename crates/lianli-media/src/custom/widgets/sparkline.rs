//! Scrolling sensor-history line graph.

use super::super::helpers::{
    fill_rounded_rect, range_color, range_color_blended, render_value_format, unit_interval,
};
use crate::common::get_exact_text_metrics;
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use lianli_shared::media::SensorRange;
use rusttype::{Font, Scale};
use std::collections::VecDeque;

pub(in super::super) struct DrawArgs<'a> {
    pub history: &'a VecDeque<f32>,
    pub value_min: f32,
    pub value_max: f32,
    pub auto_range: bool,
    pub line_color: [u8; 4],
    pub line_width: f32,
    pub fill_color: [u8; 4],
    pub fill_from_ranges: bool,
    pub range_blend: bool,
    pub background_color: [u8; 4],
    pub ranges: &'a [SensorRange],
    pub border_color: [u8; 4],
    pub border_width: f32,
    pub corner_radius: f32,
    pub padding: f32,
    pub show_points: bool,
    pub point_radius: f32,
    pub show_baseline: bool,
    pub baseline_value: f32,
    pub baseline_color: [u8; 4],
    pub baseline_width: f32,
    pub smooth: bool,
    pub scroll_rtl: bool,
    pub show_gridlines: bool,
    pub gridlines_horizontal: u32,
    pub gridlines_vertical: u32,
    pub gridline_color: [u8; 4],
    pub gridline_width: f32,
    pub show_axis_labels: bool,
    pub axis_label_count: u32,
    pub axis_labels_on_right: bool,
    pub axis_label_format: &'a str,
    pub axis_label_font: &'a Font<'static>,
    pub axis_label_size: f32,
    pub axis_label_color: [u8; 4],
    pub axis_label_padding: f32,
}

pub(in super::super) fn draw(sub: &mut RgbaImage, a: DrawArgs<'_>) {
    let (w, h) = (sub.width(), sub.height());
    if w == 0 || h == 0 {
        return;
    }

    fill_rounded_rect(sub, 0, 0, w, h, a.corner_radius, Rgba(a.background_color));

    if a.border_width > 0.0 && a.border_color[3] > 0 {
        draw_rounded_border(
            sub,
            0.0,
            0.0,
            w as f32,
            h as f32,
            a.corner_radius,
            a.border_width,
            Rgba(a.border_color),
        );
    }

    let (vmin, vmax) = compute_range(&a);

    let pad = a.padding.max(0.0);
    let mut plot_x0 = pad;
    let mut plot_y0 = pad;
    let mut plot_x1 = (w as f32 - 1.0 - pad).max(plot_x0);
    let mut plot_y1 = (h as f32 - 1.0 - pad).max(plot_y0);

    let label_scale = Scale::uniform(a.axis_label_size.max(6.0));
    let (label_reserve_x, label_reserve_y) = if a.show_axis_labels && a.axis_label_color[3] > 0 {
        let label_min = render_value_format(a.axis_label_format, vmin);
        let label_max = render_value_format(a.axis_label_format, vmax);
        let (tw_min, th_min, _, _, _) =
            get_exact_text_metrics(a.axis_label_font, &label_min, label_scale);
        let (tw_max, th_max, _, _, _) =
            get_exact_text_metrics(a.axis_label_font, &label_max, label_scale);
        let x_reserve = (tw_min.max(tw_max) as f32) + a.axis_label_padding.max(0.0) * 2.0;
        let y_reserve = (th_min.max(th_max) as f32) * 0.5 + 1.0;
        (x_reserve, y_reserve)
    } else {
        (0.0, 0.0)
    };
    if label_reserve_x > 0.0 {
        if a.axis_labels_on_right {
            plot_x1 = (plot_x1 - label_reserve_x).max(plot_x0);
        } else {
            plot_x0 = (plot_x0 + label_reserve_x).min(plot_x1);
        }
    }
    if label_reserve_y > 0.0 {
        plot_y0 = (plot_y0 + label_reserve_y).min(plot_y1);
        plot_y1 = (plot_y1 - label_reserve_y).max(plot_y0);
    }
    let plot_w = (plot_x1 - plot_x0).max(1.0);
    let plot_h = (plot_y1 - plot_y0).max(1.0);

    if a.show_gridlines && a.gridline_color[3] > 0 && a.gridline_width > 0.0 {
        let stroke = (a.gridline_width * 0.5).max(0.5);
        let col = Rgba(a.gridline_color);
        let nh = a.gridlines_horizontal;
        if nh > 0 {
            for i in 0..=nh + 1 {
                let t = i as f32 / (nh + 1) as f32;
                let y = plot_y0 + t * plot_h;
                thick_line(sub, plot_x0, y, plot_x1, y, stroke, col);
            }
        }
        let nv = a.gridlines_vertical;
        if nv > 0 {
            for i in 0..=nv + 1 {
                let t = i as f32 / (nv + 1) as f32;
                let x = plot_x0 + t * plot_w;
                thick_line(sub, x, plot_y0, x, plot_y1, stroke, col);
            }
        }
    }

    if a.show_axis_labels && a.axis_label_color[3] > 0 {
        let count = a.axis_label_count.max(2);
        let color = a.axis_label_color;
        let pad_l = a.axis_label_padding.max(0.0);
        for i in 0..count {
            let t = i as f32 / (count - 1) as f32;
            let v = vmax - t * (vmax - vmin);
            let text = render_value_format(a.axis_label_format, v);
            let (tw, th, ox, oy, _asc) =
                get_exact_text_metrics(a.axis_label_font, &text, label_scale);
            if tw <= 0 || th <= 0 {
                continue;
            }
            let y = plot_y0 + t * plot_h;
            let x = if a.axis_labels_on_right {
                plot_x1 + pad_l
            } else {
                plot_x0 - pad_l - tw as f32
            };
            let draw_x = x.round() as i32 - ox;
            let draw_y = (y - (th as f32 * 0.5)).round() as i32 - oy;
            draw_text_mut(
                sub,
                Rgba(color),
                draw_x,
                draw_y,
                label_scale,
                a.axis_label_font,
                &text,
            );
        }
    }

    if a.show_baseline && a.baseline_width > 0.0 && a.baseline_color[3] > 0 {
        let u = unit_interval(a.baseline_value, vmin, vmax);
        let y = plot_y0 + (1.0 - u) * plot_h;
        thick_line(
            sub,
            plot_x0,
            y,
            plot_x1,
            y,
            (a.baseline_width * 0.5).max(0.5),
            Rgba(a.baseline_color),
        );
    }

    let n = a.history.len();
    if n < 2 {
        return;
    }

    let mut points: Vec<(f32, f32)> = Vec::with_capacity(n);
    for (i, &v) in a.history.iter().enumerate() {
        let t = i as f32 / (n - 1) as f32;
        let x = if a.scroll_rtl {
            plot_x1 - t * plot_w
        } else {
            plot_x0 + t * plot_w
        };
        let u = unit_interval(v, vmin, vmax);
        let y = plot_y0 + (1.0 - u) * plot_h;
        points.push((x, y));
    }

    let draw_points: Vec<(f32, f32)> = if a.smooth {
        smooth_catmull_rom(&points, 8)
    } else {
        points.clone()
    };

    if a.fill_from_ranges && !a.ranges.is_empty() {
        fill_polyline_below_ranged(
            sub,
            &draw_points,
            plot_y1,
            plot_y0,
            plot_h,
            a.ranges,
            a.range_blend,
            a.fill_color[3],
        );
    } else if a.fill_color[3] > 0 {
        fill_polyline_below(sub, &draw_points, plot_y1, Rgba(a.fill_color));
    }

    let stroke_r = (a.line_width * 0.5).max(0.5);
    if !a.ranges.is_empty() && a.range_blend {
        draw_line_ranged(sub, &draw_points, plot_y0, plot_h, a.ranges, stroke_r, true);
    } else if !a.ranges.is_empty() {
        draw_line_ranged(
            sub,
            &draw_points,
            plot_y0,
            plot_h,
            a.ranges,
            stroke_r,
            false,
        );
    } else {
        for i in 1..draw_points.len() {
            let (x0, y0) = draw_points[i - 1];
            let (x1, y1) = draw_points[i];
            thick_line(sub, x0, y0, x1, y1, stroke_r, Rgba(a.line_color));
        }
    }

    if a.show_points && a.point_radius > 0.0 && a.line_color[3] > 0 {
        let color = Rgba(a.line_color);
        let r = a.point_radius;
        for (x, y) in &points {
            fill_disc(sub, *x, *y, r, color);
        }
    }
}

fn compute_range(a: &DrawArgs<'_>) -> (f32, f32) {
    if a.auto_range && a.history.len() >= 2 {
        let mut mn = f32::INFINITY;
        let mut mx = f32::NEG_INFINITY;
        for &v in a.history {
            if v < mn {
                mn = v;
            }
            if v > mx {
                mx = v;
            }
        }
        if (mx - mn).abs() < f32::EPSILON {
            (mn - 1.0, mx + 1.0)
        } else {
            let span = mx - mn;
            (mn - span * 0.05, mx + span * 0.05)
        }
    } else {
        (a.value_min, a.value_max)
    }
}

fn draw_line_ranged(
    img: &mut RgbaImage,
    points: &[(f32, f32)],
    plot_y0: f32,
    plot_h: f32,
    ranges: &[SensorRange],
    stroke_r: f32,
    blend: bool,
) {
    for i in 1..points.len() {
        let (x0, y0) = points[i - 1];
        let (x1, y1) = points[i];
        let u = 1.0 - (((y0 + y1) * 0.5 - plot_y0) / plot_h.max(1.0)).clamp(0.0, 1.0);
        let color = if blend {
            range_color_blended(ranges, u)
        } else {
            range_color(ranges, u)
        };
        thick_line(img, x0, y0, x1, y1, stroke_r, color);
    }
}

fn fill_polyline_below_ranged(
    img: &mut RgbaImage,
    points: &[(f32, f32)],
    baseline: f32,
    plot_y0: f32,
    plot_h: f32,
    ranges: &[SensorRange],
    blend: bool,
    alpha_scale: u8,
) {
    let w_i = img.width() as i32;
    let h_i = img.height() as i32;
    if w_i <= 0 || h_i <= 0 || points.len() < 2 {
        return;
    }
    let mut top: Vec<i32> = vec![i32::MAX; w_i as usize];
    for i in 1..points.len() {
        let (mut x0, mut y0) = points[i - 1];
        let (mut x1, mut y1) = points[i];
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }
        let xi0 = (x0.round() as i32).max(0);
        let xi1 = (x1.round() as i32).min(w_i - 1);
        if xi0 > xi1 {
            continue;
        }
        let dx = x1 - x0;
        for xx in xi0..=xi1 {
            let t = if dx.abs() < f32::EPSILON {
                0.0
            } else {
                ((xx as f32) - x0) / dx
            };
            let y = y0 + t * (y1 - y0);
            let yi = y.round() as i32;
            let idx = xx as usize;
            if yi < top[idx] {
                top[idx] = yi;
            }
        }
    }
    let yend = (baseline.round() as i32 + 1).min(h_i);
    let scale = if alpha_scale == 0 { 80 } else { alpha_scale } as f32 / 255.0;
    for xx in 0..w_i {
        let yi = top[xx as usize];
        if yi == i32::MAX {
            continue;
        }
        let yi = yi.max(0);
        if yi >= yend {
            continue;
        }
        for yy in yi..yend {
            let u = 1.0 - ((yy as f32 - plot_y0) / plot_h.max(1.0)).clamp(0.0, 1.0);
            let base = if blend {
                range_color_blended(ranges, u)
            } else {
                range_color(ranges, u)
            };
            let a = (base[3] as f32 * scale).round().min(255.0) as u8;
            blend_pixel(img, xx, yy, Rgba([base[0], base[1], base[2], a]));
        }
    }
}

fn smooth_catmull_rom(points: &[(f32, f32)], segments_per: usize) -> Vec<(f32, f32)> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n * segments_per);
    for i in 0..n - 1 {
        let p0 = if i == 0 { points[0] } else { points[i - 1] };
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = if i + 2 >= n {
            points[n - 1]
        } else {
            points[i + 2]
        };
        for s in 0..segments_per {
            let t = s as f32 / segments_per as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let x = 0.5
                * ((2.0 * p1.0)
                    + (-p0.0 + p2.0) * t
                    + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
                    + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
            let y = 0.5
                * ((2.0 * p1.1)
                    + (-p0.1 + p2.1) * t
                    + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                    + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
            out.push((x, y));
        }
    }
    out.push(points[n - 1]);
    out
}

fn draw_rounded_border(
    img: &mut RgbaImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    width: f32,
    color: Rgba<u8>,
) {
    let thickness = width.max(1.0);
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = x as i32;
    let y0 = y as i32;
    let x1 = (x + w).round() as i32;
    let y1 = (y + h).round() as i32;
    let r = radius.clamp(0.0, (w.min(h) * 0.5).floor());
    let inner_x0 = x + r;
    let inner_y0 = y + r;
    let inner_x1 = x + w - 1.0 - r;
    let inner_y1 = y + h - 1.0 - r;
    let r_outer = r;
    let r_inner = (r - thickness).max(0.0);
    let r_outer_sq = r_outer * r_outer;
    let r_inner_sq = r_inner * r_inner;
    for py in y0.max(0)..y1.min(ih) {
        for px in x0.max(0)..x1.min(iw) {
            let fx = px as f32;
            let fy = py as f32;
            let cx = fx.clamp(inner_x0, inner_x1);
            let cy = fy.clamp(inner_y0, inner_y1);
            let dx = fx - cx;
            let dy = fy - cy;
            let d_sq = dx * dx + dy * dy;
            let in_outer = d_sq <= r_outer_sq;
            let in_inner = d_sq <= r_inner_sq
                && fx >= x + thickness
                && fx <= x + w - 1.0 - thickness
                && fy >= y + thickness
                && fy <= y + h - 1.0 - thickness;
            if in_outer && !in_inner {
                blend_pixel(img, px, py, color);
            }
        }
    }
}

fn fill_polyline_below(img: &mut RgbaImage, points: &[(f32, f32)], baseline: f32, color: Rgba<u8>) {
    let w_i = img.width() as i32;
    let h_i = img.height() as i32;
    if w_i <= 0 || h_i <= 0 || points.len() < 2 {
        return;
    }
    let mut top: Vec<i32> = vec![i32::MAX; w_i as usize];
    for i in 1..points.len() {
        let (mut x0, mut y0) = points[i - 1];
        let (mut x1, mut y1) = points[i];
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }
        let xi0 = (x0.round() as i32).max(0);
        let xi1 = (x1.round() as i32).min(w_i - 1);
        if xi0 > xi1 {
            continue;
        }
        let dx = x1 - x0;
        for xx in xi0..=xi1 {
            let t = if dx.abs() < f32::EPSILON {
                0.0
            } else {
                ((xx as f32) - x0) / dx
            };
            let y = y0 + t * (y1 - y0);
            let yi = y.round() as i32;
            let idx = xx as usize;
            if yi < top[idx] {
                top[idx] = yi;
            }
        }
    }
    let yend = (baseline.round() as i32 + 1).min(h_i);
    for xx in 0..w_i {
        let yi = top[xx as usize];
        if yi == i32::MAX {
            continue;
        }
        let yi = yi.max(0);
        if yi >= yend {
            continue;
        }
        for yy in yi..yend {
            blend_pixel(img, xx, yy, color);
        }
    }
}

fn thick_line(
    img: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    color: Rgba<u8>,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    let steps = len.ceil().max(1.0) as i32;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let cx = x0 + t * dx;
        let cy = y0 + t * dy;
        fill_disc(img, cx, cy, radius, color);
    }
}

fn fill_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: Rgba<u8>) {
    let r_sq = r * r;
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let xmin = (cx - r).floor().max(0.0) as i32;
    let xmax = ((cx + r).ceil() as i32).min(iw - 1);
    let ymin = (cy - r).floor().max(0.0) as i32;
    let ymax = ((cy + r).ceil() as i32).min(ih - 1);
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r_sq {
                blend_pixel(img, x, y, color);
            }
        }
    }
}

fn blend_pixel(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    let a = color[3] as f32 / 255.0;
    if a <= 0.0 {
        return;
    }
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let pix = img.get_pixel_mut(x as u32, y as u32);
    pix[0] = (pix[0] as f32 * (1.0 - a) + color[0] as f32 * a).round() as u8;
    pix[1] = (pix[1] as f32 * (1.0 - a) + color[1] as f32 * a).round() as u8;
    pix[2] = (pix[2] as f32 * (1.0 - a) + color[2] as f32 * a).round() as u8;
    let alpha_out = pix[3] as f32 / 255.0 + a * (1.0 - pix[3] as f32 / 255.0);
    pix[3] = (alpha_out * 255.0).round().min(255.0) as u8;
}
