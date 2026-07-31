use fast_image_resize::{FilterType as FirFilter, ResizeAlg, ResizeOptions, Resizer};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_rect_mut;
use imageproc::rect::Rect;
use lianli_shared::media::SensorRange;
use lianli_shared::template::ImageFit;
use std::f32::consts::PI;

pub fn fast_resize_rgba(src: &RgbaImage, w: u32, h: u32, filter: FirFilter) -> RgbaImage {
    let mut dst = RgbaImage::new(w.max(1), h.max(1));
    let mut resizer = Resizer::new();
    resizer
        .resize(
            src,
            &mut dst,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter)),
        )
        .expect("fast_image_resize");
    dst
}

pub fn fit_image(src: DynamicImage, target_w: u32, target_h: u32, fit: ImageFit) -> RgbaImage {
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
            fast_overlay(&mut canvas, &rgba, ox as i64, oy as i64);
            canvas
        }
        ImageFit::Cover => {
            let resized =
                src.resize_to_fill(target_w.max(1), target_h.max(1), FilterType::Lanczos3);
            resized.to_rgba8()
        }
    }
}

pub fn range_color(ranges: &[SensorRange], unit_interval: f32) -> Rgba<u8> {
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

pub fn range_color_blended(ranges: &[SensorRange], unit_interval: f32) -> Rgba<u8> {
    if ranges.is_empty() {
        return Rgba([255, 255, 255, 255]);
    }
    let pct = unit_interval.clamp(0.0, 1.0) * 100.0;
    let stops: Vec<(f32, [u8; 4])> = {
        let mut v = Vec::with_capacity(ranges.len());
        let mut prev_max = 0.0_f32;
        for r in ranges {
            let max = r.max.unwrap_or(100.0);
            let mid = (prev_max + max) * 0.5;
            v.push((mid, [r.color[0], r.color[1], r.color[2], r.alpha]));
            prev_max = max;
        }
        v
    };
    if pct <= stops[0].0 {
        return Rgba(stops[0].1);
    }
    let last = stops.last().unwrap();
    if pct >= last.0 {
        return Rgba(last.1);
    }
    for i in 0..stops.len() - 1 {
        let (x0, c0) = stops[i];
        let (x1, c1) = stops[i + 1];
        if pct >= x0 && pct < x1 {
            let t = (pct - x0) / (x1 - x0).max(f32::EPSILON);
            return Rgba([
                lerp_u8(c0[0], c1[0], t),
                lerp_u8(c0[1], c1[1], t),
                lerp_u8(c0[2], c1[2], t),
                lerp_u8(c0[3], c1[3], t),
            ]);
        }
    }
    Rgba(last.1)
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0))
        .round()
        .clamp(0.0, 255.0) as u8
}

pub fn unit_interval(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

pub fn draw_annulus(
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

pub fn fill_rounded_rect(
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
    let max_r = (((w.min(h) as f32) - 1.0) / 2.0).floor().max(0.0);
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
pub fn fill_rect_clipped_rounded(
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
    let max_r = (((clip_w.min(clip_h) as f32) - 1.0) / 2.0).floor().max(0.0);
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

pub fn fast_overlay(dst: &mut RgbaImage, src: &RgbaImage, tl_x: i64, tl_y: i64) {
    let dw = dst.width() as i64;
    let dh = dst.height() as i64;
    let sw = src.width() as i64;
    let sh = src.height() as i64;

    let dx0 = tl_x.max(0);
    let dy0 = tl_y.max(0);
    let dx1 = (tl_x + sw).min(dw);
    let dy1 = (tl_y + sh).min(dh);
    if dx0 >= dx1 || dy0 >= dy1 {
        return;
    }

    let copy_w = (dx1 - dx0) as usize;
    let dst_stride = dst.width() as usize * 4;
    let src_stride = src.width() as usize * 4;
    let dst_x0_bytes = dx0 as usize * 4;
    let src_x0_bytes = (dx0 - tl_x) as usize * 4;
    let row_bytes = copy_w * 4;
    let dst_buf: &mut [u8] = dst.as_mut();
    let src_buf: &[u8] = src.as_raw();

    for dy in dy0..dy1 {
        let sy = (dy - tl_y) as usize;
        let dst_row = &mut dst_buf[dy as usize * dst_stride + dst_x0_bytes
            ..dy as usize * dst_stride + dst_x0_bytes + row_bytes];
        let src_row =
            &src_buf[sy * src_stride + src_x0_bytes..sy * src_stride + src_x0_bytes + row_bytes];

        for (dpx, spx) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
            let sa = spx[3] as u32;
            if sa == 0 {
                continue;
            }
            if sa == 255 {
                dpx[0] = spx[0];
                dpx[1] = spx[1];
                dpx[2] = spx[2];
                dpx[3] = 255;
                continue;
            }
            let inv = 255 - sa;
            let da = dpx[3] as u32;
            let sr = spx[0] as u32;
            let sg = spx[1] as u32;
            let sb = spx[2] as u32;
            let dr = dpx[0] as u32;
            let dg = dpx[1] as u32;
            let db = dpx[2] as u32;
            if da == 255 {
                dpx[0] = ((sr * sa + dr * inv + 127) / 255) as u8;
                dpx[1] = ((sg * sa + dg * inv + 127) / 255) as u8;
                dpx[2] = ((sb * sa + db * inv + 127) / 255) as u8;
            } else {
                let denom = sa * 255 + da * inv;
                if denom == 0 {
                    continue;
                }
                let half = denom / 2;
                dpx[0] = ((sa * sr * 255 + da * inv * dr + half) / denom) as u8;
                dpx[1] = ((sa * sg * 255 + da * inv * dg + half) / denom) as u8;
                dpx[2] = ((sa * sb * 255 + da * inv * db + half) / denom) as u8;
                dpx[3] = ((denom + 127) / 255) as u8;
            }
        }
    }
}

pub fn blit_with_opacity(dst: &mut RgbaImage, src: &RgbaImage, opacity: f32) {
    let o = opacity.clamp(0.0, 1.0);
    if o <= 0.0 {
        return;
    }
    if o >= 0.999 {
        fast_overlay(dst, src, 0, 0);
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
