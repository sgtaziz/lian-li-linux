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
