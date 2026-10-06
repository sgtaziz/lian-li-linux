use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use image::imageops::{crop_imm, overlay, resize, rotate180, rotate270, rotate90, FilterType};
use image::{RgbImage, RgbaImage};
use lianli_shared::media::MediaFraming;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::template::ImageFit;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("Media preparation cancelled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("{0}")]
    HelperTimedOut(String),
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

pub(crate) fn encode_compressed(
    tj_image: turbojpeg::Image<&[u8]>,
    screen: &ScreenInfo,
) -> Result<Vec<u8>, MediaError> {
    let buf = if screen.png {
        encode_png_bytes(&tj_image)?
    } else {
        turbojpeg::compress(
            tj_image,
            screen.jpeg_quality as i32,
            turbojpeg::Subsamp::Sub2x2,
        )
        .map_err(|e| MediaError::ImageError(format!("turbojpeg encode: {e}")))?
        .to_vec()
    };
    if buf.len() > screen.max_payload {
        return Err(MediaError::PayloadTooLarge { size: buf.len() });
    }
    Ok(buf)
}

fn encode_png_bytes(img: &turbojpeg::Image<&[u8]>) -> Result<Vec<u8>, MediaError> {
    use image::{ImageFormat, RgbImage, RgbaImage};
    use std::io::Cursor;

    let w = img.width as u32;
    let h = img.height as u32;
    let mut buf = Vec::new();
    match img.format {
        turbojpeg::PixelFormat::RGB => {
            let frame = RgbImage::from_raw(w, h, img.pixels.to_vec())
                .ok_or_else(|| MediaError::ImageError("png rgb dimensions mismatch".into()))?;
            image::DynamicImage::ImageRgb8(frame)
                .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| MediaError::ImageError(format!("png encode: {e}")))?;
        }
        turbojpeg::PixelFormat::RGBA => {
            let frame = RgbaImage::from_raw(w, h, img.pixels.to_vec())
                .ok_or_else(|| MediaError::ImageError("png rgba dimensions mismatch".into()))?;
            image::DynamicImage::ImageRgba8(frame)
                .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| MediaError::ImageError(format!("png encode: {e}")))?;
        }
        _ => {
            return Err(MediaError::ImageError(
                "unsupported pixel format for png".into(),
            ));
        }
    }
    Ok(buf)
}

pub fn frame_rgb(source: &RgbImage, framing: &MediaFraming, width: u32, height: u32) -> RgbImage {
    let framing = framing.sanitized();
    if framing.is_default() {
        return resize(source, width, height, FilterType::Lanczos3);
    }
    let crop = framing.source_crop(source.dimensions(), (width, height));
    let region = crop_imm(source, crop.x, crop.y, crop.width, crop.height).to_image();
    if framing.fit != ImageFit::Contain {
        return resize(&region, width, height, FilterType::Lanczos3);
    }
    let scale = (width as f64 / crop.width as f64).min(height as f64 / crop.height as f64);
    let fitted_w = ((crop.width as f64 * scale).round() as u32).clamp(1, width);
    let fitted_h = ((crop.height as f64 * scale).round() as u32).clamp(1, height);
    let fitted = resize(&region, fitted_w, fitted_h, FilterType::Lanczos3);
    let mut canvas = RgbImage::new(width, height);
    overlay(
        &mut canvas,
        &fitted,
        ((width - fitted_w) / 2).into(),
        ((height - fitted_h) / 2).into(),
    );
    canvas
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

/// Returns width, height, drawing offsets and ascent. Unrepresentable bounds return zeros.
pub fn get_exact_text_metrics(
    font: &FontVec,
    text: &str,
    scale: PxScale,
) -> (i32, i32, i32, i32, f32) {
    if !scale.x.is_finite() || !scale.y.is_finite() || scale.x <= 0.0 || scale.y <= 0.0 {
        return (0, 0, 0, 0, 0.0);
    }
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

    let ascent = scaled.ascent();
    match (
        max_x.checked_sub(min_x),
        max_y.checked_sub(min_y),
        (ascent as i32).checked_add(min_y),
    ) {
        (Some(width), Some(height), Some(offset_y)) => (width, height, min_x, offset_y, ascent),
        _ => (0, 0, 0, 0, 0.0),
    }
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
