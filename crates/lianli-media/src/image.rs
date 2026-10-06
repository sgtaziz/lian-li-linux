use super::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use image::{DynamicImage, ImageBuffer, ImageReader, ImageResult, Rgb};
use lianli_shared::media::MediaFraming;
use lianli_shared::screen::ScreenInfo;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(crate) fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits
}

pub(crate) fn decode_image<R: BufRead + Seek>(
    mut reader: ImageReader<R>,
) -> ImageResult<DynamicImage> {
    reader.limits(decode_limits());
    reader.decode()
}

pub(crate) fn open_image_file(path: &Path) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Image must be a regular file",
        ));
    }
    Ok(file)
}

pub(crate) fn open_image(path: &Path) -> ImageResult<DynamicImage> {
    let file = open_image_file(path)?;
    let mut reader = ImageReader::new(BufReader::new(file));
    if let Ok(format) = image::ImageFormat::from_path(path) {
        reader.set_format(format);
    }
    decode_image(reader)
}

pub fn load_image_frame(
    path: &Path,
    orientation: f32,
    framing: &MediaFraming,
    screen: &ScreenInfo,
) -> Result<Vec<u8>, MediaError> {
    let rgb = open_image(path)?.to_rgb8();
    let (w, h) = render_dimensions(screen, orientation);
    let resized = crate::common::frame_rgb(&rgb, framing, w, h);
    let oriented = apply_orientation(resized, orientation);
    encode_jpeg(oriented, screen)
}

pub fn build_color_frame(rgb: [u8; 3], screen: &ScreenInfo) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(screen.width, screen.height, Rgb(rgb));
    encode_jpeg(image, screen).expect("encoding color frame should not fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_open_rejects_nonregular_sources_before_decoding() {
        use std::io::Write;
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pipe.png");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let mut peer = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        peer.write_all(b"not a PNG header").unwrap();
        for source in [&path, &directory.path().to_path_buf()] {
            let error = open_image(source).unwrap_err();
            assert!(matches!(error, image::ImageError::IoError(ref error)
                if error.kind() == std::io::ErrorKind::InvalidInput
                && error.to_string().contains("regular file")));
        }
    }

    #[test]
    fn image_open_preserves_pixels_and_rejects_oversized_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        let expected = image::RgbaImage::from_pixel(3, 2, image::Rgba([7, 11, 19, 128]));
        expected.save(&path).unwrap();
        assert_eq!(open_image(&path).unwrap().to_rgba8(), expected);
        let link = directory.path().join("linked.png");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(open_image(&link).unwrap().to_rgba8(), expected);
        image::RgbImage::new(8193, 1).save(&path).unwrap();
        assert!(matches!(
            open_image(&path),
            Err(image::ImageError::Limits(_))
        ));
    }

    #[test]
    fn image_decode_rejects_output_over_budget_before_reading_pixels() {
        let mut header = [0u8; 54];
        header[..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&(54u32 + 8192 * 8192 * 3).to_le_bytes());
        header[10..14].copy_from_slice(&54u32.to_le_bytes());
        header[14..18].copy_from_slice(&40u32.to_le_bytes());
        header[18..22].copy_from_slice(&8192u32.to_le_bytes());
        header[22..26].copy_from_slice(&8192u32.to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());
        header[28..30].copy_from_slice(&24u16.to_le_bytes());
        let reader =
            ImageReader::with_format(std::io::Cursor::new(header), image::ImageFormat::Bmp);
        assert!(matches!(
            decode_image(reader),
            Err(image::ImageError::Limits(_))
        ));
    }

    #[test]
    fn invalid_sensor_background_is_a_preparation_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("background.png");
        std::fs::write(&path, b"invalid image").unwrap();
        let descriptor = serde_json::from_value(serde_json::json!({
            "label": "Test", "unit": "C", "source": {"type": "constant", "value": 25.0}
        }))
        .unwrap();
        let result = crate::sensor::SensorAsset::new(
            &descriptor,
            0.0,
            &ScreenInfo::WIRELESS_LCD,
            &[],
            Some(&path),
            1000,
        );
        assert!(
            matches!(result, Err(MediaError::InvalidConfig(message)) if message.contains("Sensor background image") && message.contains("background.png"))
        );
    }
}
