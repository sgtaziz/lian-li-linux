use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use image::imageops::{rotate180, rotate270, rotate90};
use image::{RgbImage, RgbaImage};
use lianli_shared::screen::ScreenInfo;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("generated frame ({size} bytes) exceeds LCD payload limit")]
    PayloadTooLarge { size: usize },
    #[error("video or animation produced no frames")]
    EmptyVideo,
    #[error("invalid fps value")]
    InvalidFps,
    #[error("sensor error: {0}")]
    Sensor(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("Background image cannot be loaded: {0}")]
    ImageError(String),
}

pub fn encode_jpeg(image: RgbImage, screen: &ScreenInfo) -> Result<Vec<u8>, MediaError> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let tj_image = turbojpeg::Image {
        pixels: image.as_raw().as_slice(),
        width,
        pitch: width * 3,
        height,
        format: turbojpeg::PixelFormat::RGB,
    };
    encode_compressed(tj_image, screen)
}

pub fn encode_jpeg_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    orientation: f32,
    screen: &ScreenInfo,
) -> Result<Vec<u8>, MediaError> {
    let orientation_q =
        (((((orientation % 360.0) + 360.0) % 360.0 + 45.0) / 90.0).floor() as i32 & 3) * 90;
    let total_rot = (orientation_q.rem_euclid(360)) as u16;

    if total_rot == 0 {
        let tj_image = turbojpeg::Image {
            pixels: rgba,
            width: width as usize,
            pitch: width as usize * 4,
            height: height as usize,
            format: turbojpeg::PixelFormat::RGBA,
        };
        return encode_compressed(tj_image, screen);
    }

    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| MediaError::ImageError("rgba bytes don't match dimensions".into()))?;
    let rotated = match total_rot {
        90 => rotate90(&img),
        180 => rotate180(&img),
        270 => rotate270(&img),
        _ => img,
    };
    let tj_image = turbojpeg::Image {
        pixels: rotated.as_raw().as_slice(),
        width: rotated.width() as usize,
        pitch: rotated.width() as usize * 4,
        height: rotated.height() as usize,
        format: turbojpeg::PixelFormat::RGBA,
    };
    encode_compressed(tj_image, screen)
}

fn encode_compressed(
    tj_image: turbojpeg::Image<&[u8]>,
    screen: &ScreenInfo,
) -> Result<Vec<u8>, MediaError> {
    let buf = turbojpeg::compress(
        tj_image,
        screen.jpeg_quality as i32,
        turbojpeg::Subsamp::Sub2x2,
    )
    .map_err(|e| MediaError::ImageError(format!("turbojpeg encode: {e}")))?
    .to_vec();
    if buf.len() > screen.max_payload {
        return Err(MediaError::PayloadTooLarge { size: buf.len() });
    }
    Ok(buf)
}

pub fn render_dimensions(screen: &ScreenInfo, orientation: f32) -> (u32, u32) {
    let norm = ((orientation % 360.0) + 360.0) % 360.0;
    if (norm - 90.0).abs() < 1.0 || (norm - 270.0).abs() < 1.0 {
        (screen.height, screen.width)
    } else {
        (screen.width, screen.height)
    }
}

pub fn apply_orientation(image: RgbImage, orientation: f32) -> RgbImage {
    let norm = ((orientation % 360.0) + 360.0) % 360.0;
    if (norm - 0.0).abs() < 0.5 || (norm - 360.0).abs() < 0.5 {
        image
    } else if (norm - 90.0).abs() < 0.5 {
        rotate90(&image)
    } else if (norm - 180.0).abs() < 0.5 {
        rotate180(&image)
    } else if (norm - 270.0).abs() < 0.5 {
        rotate270(&image)
    } else {
        let nearest = ((norm + 45.0) / 90.0).floor() as i32 & 3;
        match nearest {
            1 => rotate90(&image),
            2 => rotate180(&image),
            3 => rotate270(&image),
            _ => image,
        }
    }
}

/// Calculates how much space the text needs
/// Returns the width (tw) and height (th) of the space the text will need.
/// Additionally it returns the offsetX (ox) and offsetY (oy): If you want to fit the text into a box starting at (x/y) and extending by (tw,th), then you need to draw the text at x-ox, y-oy
///
/// But if you want the baseline of the text at box_y, you'll need to draw the text at y=box_y-ascent: So if you want to draw several characters each after another, you need to keep the baseline constant.
/// If you draw a text at x/y, then the baseline will be at y+ascent. The topmost coord will be at y+oy and the bottommost coord will be y+oy+th-1. The text will NOT appear at x/y, as this coord is only the top left coord of the glyph (which in almost all cases starts with an offset).

pub fn get_exact_text_metrics(
    font: &FontVec,
    text: &str,
    scale: PxScale,
) -> (i32, i32, i32, i32, f32) {
    let scaled = font.as_scaled(scale);

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;

    let mut cursor_x = 0.0_f32;
    for ch in text.chars() {
        let glyph_id = scaled.glyph_id(ch);
        let glyph = glyph_id.with_scale_and_position(scale, point(cursor_x, 0.0));
        if let Some(outlined) = scaled.outline_glyph(glyph) {
            let bb = outlined.px_bounds();
            if (bb.min.x as i32) < min_x {
                min_x = bb.min.x as i32;
            }
            if (bb.min.y as i32) < min_y {
                min_y = bb.min.y as i32;
            }
            if (bb.max.x as i32) > max_x {
                max_x = bb.max.x as i32;
            }
            if (bb.max.y as i32) > max_y {
                max_y = bb.max.y as i32;
            }
        }
        cursor_x += scaled.h_advance(glyph_id);
    }

    if max_x < min_x || max_y < min_y {
        return (0, 0, 0, 0, 0.0);
    }

    let width = max_x - min_x;
    let height = max_y - min_y;
    let ascent = scaled.ascent();

    (width, height, min_x, (ascent as i32) + min_y, ascent)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r_temp, g_temp, b_temp) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    [
        ((r_temp + m) * 255.0).round() as u8,
        ((g_temp + m) * 255.0).round() as u8,
        ((b_temp + m) * 255.0).round() as u8,
    ]
}
