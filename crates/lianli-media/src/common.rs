use image::imageops::{rotate180, rotate270, rotate90};
use image::RgbImage;
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
}

pub fn encode_jpeg(image: RgbImage, screen: &ScreenInfo) -> Result<Vec<u8>, MediaError> {
    let final_image = apply_device_rotation(image, screen.device_rotation);
    let mut buf = Vec::new();
    {
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, screen.jpeg_quality);
        encoder.encode_image(&final_image)?;
    }
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
