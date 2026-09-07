use image::{ImageBuffer, Rgb, RgbImage};
use lianli_media::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use lianli_shared::screen::ScreenInfo;

fn test_screen() -> ScreenInfo {
    ScreenInfo {
        width: 480,
        height: 1920,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 512_000,
        h264: false,
        needs_keepalive: false,
        png: false,
        play_count: 0,
    }
}

#[test]
fn render_dimensions_portrait_unchanged() {
    let screen = test_screen();
    assert_eq!(render_dimensions(&screen, 0.0), (480, 1920));
    assert_eq!(render_dimensions(&screen, 180.0), (480, 1920));
}

#[test]
fn render_dimensions_landscape_swapped() {
    let screen = test_screen();
    assert_eq!(render_dimensions(&screen, 90.0), (1920, 480));
    assert_eq!(render_dimensions(&screen, 270.0), (1920, 480));
}

#[test]
fn render_dimensions_handles_negative_orientation() {
    let screen = test_screen();
    assert_eq!(render_dimensions(&screen, -90.0), (1920, 480));
    assert_eq!(render_dimensions(&screen, -180.0), (480, 1920));
}

#[test]
fn render_dimensions_handles_overflow() {
    let screen = test_screen();
    assert_eq!(render_dimensions(&screen, 450.0), (1920, 480));
    assert_eq!(render_dimensions(&screen, 360.0), (480, 1920));
}

#[test]
fn apply_orientation_0_is_identity() {
    let img: RgbImage = ImageBuffer::from_pixel(4, 6, Rgb([1, 2, 3]));
    let result = apply_orientation(img, 0.0);
    assert_eq!(result.dimensions(), (4, 6));
}

#[test]
fn apply_orientation_90_swaps_dimensions() {
    let img: RgbImage = ImageBuffer::from_pixel(4, 6, Rgb([1, 2, 3]));
    let result = apply_orientation(img, 90.0);
    assert_eq!(result.dimensions(), (6, 4));
}

#[test]
fn apply_orientation_180_preserves_dimensions() {
    let img: RgbImage = ImageBuffer::from_pixel(4, 6, Rgb([1, 2, 3]));
    let result = apply_orientation(img, 180.0);
    assert_eq!(result.dimensions(), (4, 6));
}

#[test]
fn apply_orientation_270_swaps_dimensions() {
    let img: RgbImage = ImageBuffer::from_pixel(4, 6, Rgb([1, 2, 3]));
    let result = apply_orientation(img, 270.0);
    assert_eq!(result.dimensions(), (6, 4));
}

#[test]
fn apply_orientation_snap_to_nearest_90() {
    let img: RgbImage = ImageBuffer::from_pixel(4, 6, Rgb([1, 2, 3]));
    assert_eq!(apply_orientation(img.clone(), 44.0).dimensions(), (4, 6));
    assert_eq!(apply_orientation(img.clone(), 46.0).dimensions(), (6, 4));
    assert_eq!(apply_orientation(img.clone(), 134.0).dimensions(), (6, 4));
    assert_eq!(apply_orientation(img, 136.0).dimensions(), (4, 6));
}

#[test]
fn encode_compressed_png_when_screen_uses_png() {
    let mut screen = test_screen();
    screen.png = true;
    let img: RgbImage = ImageBuffer::from_pixel(2, 2, Rgb([255, 0, 0]));
    let result = encode_jpeg(img, &screen).unwrap();
    assert_eq!(
        &result[..8],
        &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    );
}

#[test]
fn encode_compressed_jpeg_when_screen_uses_jpeg() {
    let screen = test_screen();
    let img: RgbImage = ImageBuffer::from_pixel(2, 2, Rgb([255, 0, 0]));
    let result = encode_jpeg(img, &screen).unwrap();
    assert_eq!(&result[..2], &[0xFF, 0xD8]);
}

#[test]
fn encode_compressed_rejects_oversized_payload() {
    let mut screen = test_screen();
    screen.max_payload = 1;
    let img: RgbImage = ImageBuffer::from_pixel(100, 100, Rgb([255, 128, 0]));
    let result = encode_jpeg(img, &screen);
    assert!(matches!(result, Err(MediaError::PayloadTooLarge { .. })));
}

#[test]
fn prepare_media_asset_image_produces_static_even_when_h264_screen() {
    let mut screen = test_screen();
    screen.h264 = true;
    let temp = tempfile::TempDir::new().unwrap();
    let img_path = temp.path().join("test.png");
    let img: RgbImage = ImageBuffer::from_pixel(16, 16, Rgb([0, 255, 0]));
    img.save(&img_path).unwrap();

    let cfg = lianli_shared::config::LcdConfig {
        index: None,
        serial: None,
        media_type: lianli_shared::media::MediaType::Image,
        path: Some(img_path),
        fps: Some(30.0),
        update_interval_ms: None,
        rgb: None,
        orientation: 0.0,
        sensor: None,
        sensor_source_1: Default::default(),
        sensor_source_2: Default::default(),
        doublegauge: None,
        template_id: None,
        smooth_edges: None,
        custom_h264: None,
        aio_512_frame: None,
        brightness: None,
    };

    let asset =
        lianli_media::prepare_media_asset(&cfg, 30.0, &screen, screen.h264, &[], &[]).unwrap();
    assert!(matches!(asset, lianli_media::MediaAssetKind::Static { .. }));
}

#[test]
fn prepare_media_asset_color_produces_static_even_when_h264_screen() {
    let mut screen = test_screen();
    screen.h264 = true;
    let cfg = lianli_shared::config::LcdConfig {
        index: None,
        serial: None,
        media_type: lianli_shared::media::MediaType::Color,
        path: None,
        fps: Some(30.0),
        update_interval_ms: None,
        rgb: Some([255, 0, 0]),
        orientation: 0.0,
        sensor: None,
        sensor_source_1: Default::default(),
        sensor_source_2: Default::default(),
        doublegauge: None,
        template_id: None,
        smooth_edges: None,
        custom_h264: None,
        aio_512_frame: None,
        brightness: None,
    };

    let asset =
        lianli_media::prepare_media_asset(&cfg, 30.0, &screen, screen.h264, &[], &[]).unwrap();
    assert!(matches!(asset, lianli_media::MediaAssetKind::Static { .. }));
}
