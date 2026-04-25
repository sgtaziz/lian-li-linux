use super::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use image::imageops::FilterType;
use image::{ImageBuffer, Rgb};
use lianli_shared::screen::ScreenInfo;
use std::path::Path;

pub fn load_image_frame(
    path: &Path,
    orientation: f32,
    screen: &ScreenInfo,
) -> Result<Vec<u8>, MediaError> {
    let rgb = image::open(path)?.to_rgb8();
    let (w, h) = render_dimensions(screen, orientation);
    let resized = image::imageops::resize(&rgb, w, h, FilterType::Lanczos3);
    let oriented = apply_orientation(resized, orientation);
    encode_jpeg(oriented, screen)
}

pub fn build_color_frame(rgb: [u8; 3], screen: &ScreenInfo) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(screen.width, screen.height, Rgb(rgb));
    encode_jpeg(image, screen).expect("encoding color frame should not fail")
}

pub fn encode_aio_image(path: &Path) -> Result<Vec<u8>, MediaError> {
    use image::GenericImageView;
    let img = image::open(path)?;
    let target = lianli_shared::aio::AIO_PIC_DIMENSION;
    let (w, h) = img.dimensions();
    let resized = if w == target && h == target {
        img.to_rgb8()
    } else {
        img.resize_exact(target, target, FilterType::Lanczos3)
            .to_rgb8()
    };
    for quality in [85u8, 75, 65, 55, 45, 35] {
        let mut buf: Vec<u8> = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        encoder
            .encode(
                resized.as_raw(),
                resized.width(),
                resized.height(),
                image::ColorType::Rgb8,
            )
            .map_err(image::ImageError::from)?;
        if buf.len() <= lianli_shared::aio::AIO_PIC_MAX_BYTES {
            return Ok(buf);
        }
    }
    Err(MediaError::Sensor(format!(
        "image too large even at quality 35 (max {} bytes)",
        lianli_shared::aio::AIO_PIC_MAX_BYTES
    )))
}
