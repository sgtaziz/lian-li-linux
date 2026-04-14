use image::imageops::{rotate180, rotate270, rotate90};
use image::RgbImage;
use lianli_shared::screen::ScreenInfo;
use rusttype::{point, Font, Scale};
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
    let final_image = apply_device_rotation(image, screen.device_rotation);
    let width = final_image.width() as usize;
    let height = final_image.height() as usize;
    let tj_image = turbojpeg::Image {
        pixels: final_image.as_raw().as_slice(),
        width,
        pitch: width * 3,
        height,
        format: turbojpeg::PixelFormat::RGB,
    };
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

fn apply_device_rotation(image: RgbImage, rotation: u16) -> RgbImage {
    match rotation {
        90 => rotate90(&image),
        180 => rotate180(&image),
        270 => rotate270(&image),
        _ => image,
    }
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

pub fn get_exact_text_metrics(font: &Font, text: &str, scale: Scale) -> (i32, i32, i32, i32, f32) {
    let glyphs: Vec<_> = font.layout(text, scale, point(0.0, 0.0)).collect();

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;

    for glyph in glyphs {
        if let Some(bb) = glyph.pixel_bounding_box() {
            if bb.min.x < min_x {
                min_x = bb.min.x;
            }
            if bb.min.y < min_y {
                min_y = bb.min.y;
            }
            if bb.max.x > max_x {
                max_x = bb.max.x;
            }
            if bb.max.y > max_y {
                max_y = bb.max.y;
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        return (0, 0, 0, 0, 0.0);
    }

    // Width and height of the pixels actually occupied by the glyphs.
    let width = max_x - min_x;
    let height = max_y - min_y;

    // min_x and min_y are the pixel offsets from the anchor point (0, 0).
    // min_y is almost always negative for text (glyphs sit above the baseline).

    let v_metrics = font.v_metrics(scale);

    (
        width,
        height,
        min_x,
        (v_metrics.ascent as i32) + min_y,
        v_metrics.ascent,
    )
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
