//! Shared helpers for Custom widget rendering.

use crate::common::{get_exact_text_metrics, MediaError};
use image::imageops::FilterType;
use image::{imageops, DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::sensors::{resolve_sensor, ResolvedSensor, SensorInfo};
use lianli_shared::template::{FontRef, ImageFit, TextAlign, Widget, WidgetKind};
use rusttype::{Font, Scale};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};

/// Monotonic elapsed-ms since asset creation, used for video playback timing.
#[derive(Copy, Clone)]
pub(super) struct ElapsedMs(pub u64);

impl From<u64> for ElapsedMs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

pub(super) fn widget_sensor_source(kind: &WidgetKind) -> Option<&SensorSourceConfig> {
    match kind {
        WidgetKind::ValueText { source, .. }
        | WidgetKind::RadialGauge { source, .. }
        | WidgetKind::VerticalBar { source, .. }
        | WidgetKind::HorizontalBar { source, .. }
        | WidgetKind::Speedometer { source, .. } => Some(source),
        _ => None,
    }
}

pub(super) fn resolve_sensor_source(
    source: &SensorSourceConfig,
    all_sensors: &[SensorInfo],
) -> Option<ResolvedSensor> {
    if let SensorSourceConfig::Constant { value } = source {
        return Some(ResolvedSensor::Constant(*value));
    }
    let target = source.to_sensor_source();
    let divider = all_sensors
        .iter()
        .find(|s| s.source == target)
        .map(|s| s.divider)
        .unwrap_or(1);
    resolve_sensor(&target, divider)
}

pub(super) fn widget_size_px(widget: &Widget, uniform_scale: f32) -> (u32, u32) {
    (
        (widget.width * uniform_scale).round().max(1.0) as u32,
        (widget.height * uniform_scale).round().max(1.0) as u32,
    )
}

pub(super) fn format_sensor_readout(kind: &WidgetKind, raw: f32) -> (String, i32) {
    match kind {
        WidgetKind::ValueText { format, unit, .. } => {
            let text = render_value_format(format, raw);
            let quantized = (raw * 10.0).round() as i32;
            (format!("{text}{unit}"), quantized)
        }
        WidgetKind::RadialGauge {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::VerticalBar {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::HorizontalBar {
            value_min,
            value_max,
            ..
        }
        | WidgetKind::Speedometer {
            value_min,
            value_max,
            ..
        } => {
            let span = (value_max - value_min).abs().max(f32::EPSILON);
            let q = (((raw - value_min) / span) * 1000.0).round() as i32;
            (String::new(), q)
        }
        _ => (String::new(), 0),
    }
}

fn render_value_format(fmt: &str, value: f32) -> String {
    if let Some(rest) = fmt.strip_prefix("{:.") {
        if let Some(n_str) = rest.strip_suffix("}") {
            if let Ok(n) = n_str.parse::<usize>() {
                return format!("{:.*}", n, value);
            }
        }
    }
    if fmt == "{}" {
        return format!("{:.0}", value);
    }
    if let Some(pos) = fmt.find("{}") {
        let mut out = String::with_capacity(fmt.len() + 8);
        out.push_str(&fmt[..pos]);
        out.push_str(&format!("{:.0}", value));
        out.push_str(&fmt[pos + 2..]);
        return out;
    }
    format!("{:.0}", value)
}

pub(super) fn fit_image(
    src: DynamicImage,
    target_w: u32,
    target_h: u32,
    fit: ImageFit,
) -> RgbaImage {
    match fit {
        ImageFit::Stretch => src
            .resize_exact(target_w.max(1), target_h.max(1), FilterType::Lanczos3)
            .to_rgba8(),
        ImageFit::Contain => {
            let resized = src.resize(target_w.max(1), target_h.max(1), FilterType::Lanczos3);
            let mut canvas =
                RgbaImage::from_pixel(target_w.max(1), target_h.max(1), Rgba([0, 0, 0, 0]));
            let rgba = resized.to_rgba8();
            let ox = ((target_w as i32) - (rgba.width() as i32)) / 2;
            let oy = ((target_h as i32) - (rgba.height() as i32)) / 2;
            imageops::overlay(&mut canvas, &rgba, ox as i64, oy as i64);
            canvas
        }
        ImageFit::Cover => {
            let resized =
                src.resize_to_fill(target_w.max(1), target_h.max(1), FilterType::Lanczos3);
            resized.to_rgba8()
        }
    }
}

pub(super) fn load_font_from_disk(path: &Path) -> Result<Font<'static>, MediaError> {
    let bytes = std::fs::read(path)
        .map_err(|e| MediaError::Sensor(format!("font '{}' read failed: {e}", path.display())))?;
    Font::try_from_vec(bytes)
        .ok_or_else(|| MediaError::Sensor(format!("font '{}' parse failed", path.display())))
}

pub(super) fn widget_font_ref(kind: &WidgetKind) -> Option<&FontRef> {
    match kind {
        WidgetKind::Label { font, .. } | WidgetKind::ValueText { font, .. } => Some(font),
        _ => None,
    }
}

pub(super) fn resolve_font<'a>(
    font_ref: &FontRef,
    fonts: &'a HashMap<PathBuf, Font<'static>>,
    default: &'a Font<'static>,
) -> &'a Font<'static> {
    if let Some(p) = &font_ref.path {
        if let Some(f) = fonts.get(p) {
            return f;
        }
    }
    default
}

pub(super) fn range_color(ranges: &[SensorRange], unit_interval: f32) -> Rgba<u8> {
    if ranges.is_empty() {
        return Rgba([255, 255, 255, 255]);
    }
    let pct = unit_interval.clamp(0.0, 1.0) * 100.0;
    for r in ranges {
        if let Some(max) = r.max {
            if pct <= max {
                return Rgba([r.color[0], r.color[1], r.color[2], r.alpha]);
            }
        } else {
            return Rgba([r.color[0], r.color[1], r.color[2], r.alpha]);
        }
    }
    let last = ranges.last().unwrap();
    Rgba([last.color[0], last.color[1], last.color[2], last.alpha])
}

pub(super) fn unit_interval(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

pub(super) fn draw_annulus(
    img: &mut RgbaImage,
    center: (f32, f32),
    r_in: f32,
    r_out: f32,
    start_deg: f32,
    sweep_deg: f32,
    color: Rgba<u8>,
) {
    if color[3] == 0 {
        return;
    }
    let r_in_sq = r_in * r_in;
    let r_out_sq = r_out * r_out;
    let start_rad = start_deg.to_radians();
    let sweep_rad = sweep_deg.to_radians();
    let (w, h) = (img.width(), img.height());
    let xmin = (center.0 - r_out).floor().max(0.0) as u32;
    let xmax = ((center.0 + r_out).ceil() as u32).min(w);
    let ymin = (center.1 - r_out).floor().max(0.0) as u32;
    let ymax = ((center.1 + r_out).ceil() as u32).min(h);

    for y in ymin..ymax {
        for x in xmin..xmax {
            let dx = x as f32 - center.0;
            let dy = y as f32 - center.1;
            let d_sq = dx * dx + dy * dy;
            if d_sq < r_in_sq || d_sq > r_out_sq {
                continue;
            }
            let mut theta = dy.atan2(dx) - start_rad;
            while theta < 0.0 {
                theta += 2.0 * PI;
            }
            while theta >= 2.0 * PI {
                theta -= 2.0 * PI;
            }
            let sweep_norm = if sweep_rad >= 0.0 {
                sweep_rad.min(2.0 * PI)
            } else {
                (2.0 * PI) + sweep_rad.max(-2.0 * PI)
            };
            if theta <= sweep_norm {
                img.put_pixel(x, y, color);
            }
        }
    }
}

pub(super) fn fill_rounded_rect(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: f32,
    color: Rgba<u8>,
) {
    if color[3] == 0 || w == 0 || h == 0 {
        return;
    }
    let max_r = (w.min(h) as f32 / 2.0).floor();
    let r = radius.clamp(0.0, max_r);
    if r <= 0.5 {
        draw_filled_rect_mut(img, Rect::at(x, y).of_size(w, h), color);
        return;
    }
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(iw);
    let y1 = (y + h as i32).min(ih);
    let inner_x0 = x as f32 + r;
    let inner_y0 = y as f32 + r;
    let inner_x1 = x as f32 + w as f32 - 1.0 - r;
    let inner_y1 = y as f32 + h as f32 - 1.0 - r;
    let r_sq = r * r;
    for py in y0..y1 {
        for px in x0..x1 {
            let fx = px as f32;
            let fy = py as f32;
            let cx = fx.clamp(inner_x0, inner_x1);
            let cy = fy.clamp(inner_y0, inner_y1);
            let dx = fx - cx;
            let dy = fy - cy;
            if dx * dx + dy * dy <= r_sq {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fill_rect_clipped_rounded(
    img: &mut RgbaImage,
    rect_x: i32,
    rect_y: i32,
    rect_w: u32,
    rect_h: u32,
    clip_x: i32,
    clip_y: i32,
    clip_w: u32,
    clip_h: u32,
    clip_radius: f32,
    color: Rgba<u8>,
) {
    if color[3] == 0 || rect_w == 0 || rect_h == 0 || clip_w == 0 || clip_h == 0 {
        return;
    }
    let max_r = (clip_w.min(clip_h) as f32 / 2.0).floor();
    let r = clip_radius.clamp(0.0, max_r);
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = rect_x.max(clip_x).max(0);
    let y0 = rect_y.max(clip_y).max(0);
    let x1 = (rect_x + rect_w as i32).min(clip_x + clip_w as i32).min(iw);
    let y1 = (rect_y + rect_h as i32).min(clip_y + clip_h as i32).min(ih);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    if r <= 0.5 {
        let w = (x1 - x0) as u32;
        let h = (y1 - y0) as u32;
        draw_filled_rect_mut(img, Rect::at(x0, y0).of_size(w, h), color);
        return;
    }
    let inner_x0 = clip_x as f32 + r;
    let inner_y0 = clip_y as f32 + r;
    let inner_x1 = clip_x as f32 + clip_w as f32 - 1.0 - r;
    let inner_y1 = clip_y as f32 + clip_h as f32 - 1.0 - r;
    let r_sq = r * r;
    for py in y0..y1 {
        for px in x0..x1 {
            let fx = px as f32;
            let fy = py as f32;
            let cx = fx.clamp(inner_x0, inner_x1);
            let cy = fy.clamp(inner_y0, inner_y1);
            let dx = fx - cx;
            let dy = fy - cy;
            if dx * dx + dy * dy <= r_sq {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

pub(super) fn blit_with_opacity(dst: &mut RgbaImage, src: &RgbaImage, opacity: f32) {
    let o = opacity.clamp(0.0, 1.0);
    if o <= 0.0 {
        return;
    }
    if o >= 0.999 && src.width() == dst.width() && src.height() == dst.height() {
        imageops::overlay(dst, src, 0, 0);
        return;
    }
    let (dw, dh) = (dst.width(), dst.height());
    let (sw, sh) = (src.width(), src.height());
    let w = sw.min(dw);
    let h = sh.min(dh);
    for y in 0..h {
        for x in 0..w {
            let s = src.get_pixel(x, y);
            let d = dst.get_pixel_mut(x, y);
            let a = (s[3] as f32 / 255.0) * o;
            d[0] = (d[0] as f32 * (1.0 - a) + s[0] as f32 * a).round() as u8;
            d[1] = (d[1] as f32 * (1.0 - a) + s[1] as f32 * a).round() as u8;
            d[2] = (d[2] as f32 * (1.0 - a) + s[2] as f32 * a).round() as u8;
        }
    }
}

pub(super) fn draw_text_widget(
    sub: &mut RgbaImage,
    text: &str,
    font: &Font<'static>,
    size: f32,
    color: [u8; 4],
    align: TextAlign,
    ww: u32,
    wh: u32,
) {
    if text.is_empty() || color[3] == 0 {
        return;
    }
    let scale = Scale::uniform(size.max(1.0));
    let (tw, th, ox, oy, _ascent) = get_exact_text_metrics(font, text, scale);
    if tw <= 0 || th <= 0 {
        return;
    }
    let x = match align {
        TextAlign::Left => 0,
        TextAlign::Center => ((ww as i32) - tw) / 2,
        TextAlign::Right => (ww as i32) - tw,
    } - ox;
    let y = ((wh as i32) - th) / 2 - oy;
    draw_text_mut(sub, Rgba(color), x, y, scale, font, text);
}
