pub mod ffmpeg;
mod frame_budget;
pub mod h264;
pub mod h264_inprocess;
pub mod h264_live;
pub(crate) mod process;
pub mod vaapi;
pub(crate) mod widget_animation;
pub(crate) mod widget_stream;

pub use ffmpeg::{cap_fps_to_source, probe_source_fps};
pub use h264::encode_h264;
pub use h264::encode_h264_with_status;
pub use h264_inprocess::{ensure_ffmpeg_initialized, H264Encoder};
pub use h264_live::LiveH264Encoder;

use crate::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use crate::PreparationControl;
use ffmpeg::stream_rgba;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, Frames, ImageDecoder, RgbaImage};
use lianli_shared::media::MediaFraming;
use lianli_shared::screen::ScreenInfo;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

pub fn build_video_frames(
    path: &Path,
    fps: f32,
    orientation: f32,
    framing: &MediaFraming,
    screen: &ScreenInfo,
    control: impl Into<PreparationControl>,
) -> Result<(Vec<Vec<u8>>, Vec<Duration>), MediaError> {
    let control = control.into();
    control.check()?;
    let (rw, rh) = render_dimensions(screen, orientation);
    let mut frames = Vec::new();
    let mut budget = frame_budget::FrameBudget::default();
    stream_rgba(path, fps, rw, rh, framing, &control, |rgba| {
        control.check()?;
        let frame = crate::common::encode_jpeg_rgba(rgba, rw, rh, orientation, screen)?;
        budget.reserve(frame.len())?;
        frames.push(frame);
        Ok(())
    })?;
    if frames.is_empty() {
        return Err(MediaError::EmptyVideo);
    }

    let interval = Duration::from_secs_f32(1.0 / fps);
    let durations = vec![interval; frames.len()];
    Ok((frames, durations))
}

pub fn build_gif_frames(
    path: &Path,
    orientation: f32,
    screen: &ScreenInfo,
    desired_fps: Option<f32>,
) -> Result<(Vec<Vec<u8>>, Vec<Duration>), MediaError> {
    build_gif_frames_cancellable(
        path,
        orientation,
        screen,
        desired_fps,
        &MediaFraming::default(),
        &PreparationControl::new(false),
    )
}

pub(crate) fn build_gif_frames_cancellable(
    path: &Path,
    orientation: f32,
    screen: &ScreenInfo,
    desired_fps: Option<f32>,
    framing: &MediaFraming,
    control: &PreparationControl,
) -> Result<(Vec<Vec<u8>>, Vec<Duration>), MediaError> {
    control.check()?;
    let file = BufReader::new(crate::image::open_image_file(path)?);
    let mut decoder = GifDecoder::new(file)?;
    decoder.set_limits(crate::image::decode_limits())?;
    let (rw, rh) = render_dimensions(screen, orientation);
    frame_budget::rgba_bytes(rw, rh)?;
    let mut budget = frame_budget::FrameBudget::default();
    let mut encoded = Vec::new();
    let mut durations = Vec::new();

    let target_ms = desired_fps.map(|fps| 1000.0 / fps.max(1.0));
    let mut accum_ms = 0.0f32;

    let mut frames = decoder.into_frames().peekable();
    while let Some(frame) = {
        control.check()?;
        frames.next()
    } {
        let frame = frame?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let native_ms = if denom == 0 {
            numer as f32
        } else {
            numer as f32 / denom as f32
        };
        let native_ms = native_ms.max(10.0);
        accum_ms += native_ms;

        control.check()?;
        let is_last = frames.peek().is_none();
        let should_emit = match target_ms {
            Some(t) => accum_ms >= t || is_last,
            None => true,
        };
        if !should_emit {
            continue;
        }

        let rgba = frame.into_buffer();
        let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
        let resized = crate::common::frame_rgb(&rgb, framing, rw, rh);
        let oriented = apply_orientation(resized, orientation);
        let jpeg = encode_jpeg(oriented, screen)?;
        budget.reserve(jpeg.len())?;
        encoded.push(jpeg);
        durations.push(Duration::from_millis(accum_ms as u64));
        accum_ms = 0.0;
    }

    if encoded.is_empty() {
        return Err(MediaError::EmptyVideo);
    }

    Ok((encoded, durations))
}

pub fn decode_frames_to_rgba(
    path: &Path,
    fps: f32,
    width: u32,
    height: u32,
    control: impl Into<PreparationControl>,
) -> Result<(Vec<RgbaImage>, Vec<Duration>), MediaError> {
    let control = control.into();
    control.check()?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "gif" {
        let mut decoder = GifDecoder::new(BufReader::new(crate::image::open_image_file(path)?))?;
        decoder.set_limits(crate::image::decode_limits())?;
        return decode_animation_frames(decoder.into_frames(), width, height, &control);
    }

    if ext == "png" || ext == "apng" {
        let mut decoder = PngDecoder::with_limits(
            BufReader::new(crate::image::open_image_file(path)?),
            crate::image::decode_limits(),
        )?;
        if decoder.is_apng()? {
            let apng = decoder.apng()?;
            return decode_animation_frames(apng.into_frames(), width, height, &control);
        }
        frame_budget::rgba_bytes(width, height)?;
        let mut limits = crate::image::decode_limits();
        limits.reserve(decoder.total_bytes())?;
        decoder.set_limits(limits)?;
        let img = DynamicImage::from_decoder(decoder)?;
        let resized = image::imageops::resize(&img.to_rgba8(), width, height, FilterType::Lanczos3);
        return Ok((vec![resized], vec![Duration::from_millis(100)]));
    }

    let mut frames = Vec::new();
    let mut budget = frame_budget::FrameBudget::default();
    stream_rgba(
        path,
        fps,
        width,
        height,
        &MediaFraming::default(),
        &control,
        |rgba| {
            control.check()?;
            budget.reserve(rgba.len())?;
            let frame = RgbaImage::from_raw(width, height, rgba.to_vec())
                .ok_or_else(|| MediaError::ImageError("Invalid decoded RGBA frame size".into()))?;
            frames.push(frame);
            Ok(())
        },
    )?;
    if frames.is_empty() {
        return Err(MediaError::EmptyVideo);
    }

    let interval = Duration::from_secs_f32(1.0 / fps.max(1.0));
    let durations = vec![interval; frames.len()];
    Ok((frames, durations))
}

fn decode_animation_frames(
    mut frames: Frames<'_>,
    width: u32,
    height: u32,
    control: &PreparationControl,
) -> Result<(Vec<RgbaImage>, Vec<Duration>), MediaError> {
    let frame_bytes = frame_budget::rgba_bytes(width, height)?;
    let mut budget = frame_budget::FrameBudget::default();
    let mut out_frames = Vec::new();
    let mut durations = Vec::new();
    while let Some(frame) = {
        control.check()?;
        frames.next()
    } {
        let frame = frame?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let millis = if denom == 0 {
            numer as f32
        } else {
            numer as f32 / denom as f32
        };
        let duration = Duration::from_millis(millis.max(10.0) as u64);
        budget.reserve(frame_bytes)?;
        let rgba = frame.into_buffer();
        let resized = image::imageops::resize(&rgba, width, height, FilterType::Lanczos3);
        out_frames.push(resized);
        durations.push(duration);
    }
    if out_frames.is_empty() {
        return Err(MediaError::EmptyVideo);
    }
    Ok((out_frames, durations))
}

pub(super) fn target_dimensions(screen: &ScreenInfo, orientation: f32) -> (u32, u32) {
    let (rw, rh) = render_dimensions(screen, orientation);
    let rot = (orientation % 360.0 + 360.0) % 360.0;
    if (rot - 90.0).abs() < 1.0 || (rot - 270.0).abs() < 1.0 {
        (rh, rw)
    } else {
        (rw, rh)
    }
}

#[cfg(test)]
mod animation_budget_tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn animation_sources_reject_nonregular_files_before_decoding() {
        let root = tempfile::tempdir().unwrap();
        for extension in ["gif", "png", "apng"] {
            let path = root.path().join(format!("directory.{extension}"));
            std::fs::create_dir(&path).unwrap();
            let result = decode_frames_to_rgba(&path, 30.0, 8, 8, false);
            assert!(result.unwrap_err().to_string().contains("regular file"));
            if extension == "gif" {
                let result = build_gif_frames(&path, 0.0, &ScreenInfo::WIRELESS_LCD, None);
                assert!(result.unwrap_err().to_string().contains("regular file"));
            }
        }
    }

    #[test]
    fn streamed_video_preserves_pixels_geometry_timing_and_payload_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("video.bin");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    RgbaImage::from_pixel(16, 8, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let screen = ScreenInfo {
            width: 8,
            height: 16,
            ..ScreenInfo::WIRELESS_LCD
        };
        let (frames, durations) =
            build_video_frames(&path, 10.0, 90.0, &MediaFraming::default(), &screen, false)
                .unwrap();
        assert_eq!(durations.len(), 2);
        assert!(durations
            .iter()
            .all(|duration| duration.as_micros() == 100_000));
        let first = image::load_from_memory(&frames[0]).unwrap().to_rgb8();
        let second = image::load_from_memory(&frames[1]).unwrap().to_rgb8();
        assert_eq!(first.dimensions(), (8, 16));
        assert!(first.get_pixel(0, 0)[0] > 240);
        assert!(second.get_pixel(0, 0)[2] > 240);
        let png_screen = ScreenInfo {
            png: true,
            ..screen
        };
        let (frames, _) = build_video_frames(
            &path,
            10.0,
            0.0,
            &MediaFraming::default(),
            &png_screen,
            false,
        )
        .unwrap();
        assert_eq!(
            image::guess_format(&frames[0]).unwrap(),
            image::ImageFormat::Png
        );
        let (frames, durations) = decode_frames_to_rgba(&path, 10.0, 16, 8, false).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(durations.len(), 2);
        assert!(durations
            .iter()
            .all(|duration| duration.as_micros() == 100_000));
        assert_eq!(frames[0].get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frames[1].get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert!(matches!(
            build_video_frames(
                &path,
                f32::NAN,
                0.0,
                &MediaFraming::default(),
                &screen,
                false
            ),
            Err(MediaError::InvalidFps)
        ));
    }

    fn frame(color: u8, millis: u32) -> image::Frame {
        image::Frame::from_parts(
            RgbaImage::from_pixel(1, 1, image::Rgba([color, 0, 0, 255])),
            0,
            0,
            image::Delay::from_numer_denom_ms(millis, 1),
        )
    }

    #[test]
    fn animation_preserves_pixels_delays_and_rejects_excess_frames() {
        let source = vec![Ok(frame(10, 25)), Ok(frame(20, 75))];
        let (frames, delays) = decode_animation_frames(
            Frames::new(Box::new(source.into_iter())),
            1,
            1,
            &PreparationControl::new(false),
        )
        .unwrap();
        assert_eq!(frames[0].get_pixel(0, 0).0, [10, 0, 0, 255]);
        assert_eq!(frames[1].get_pixel(0, 0).0, [20, 0, 0, 255]);
        assert_eq!(
            delays,
            vec![Duration::from_millis(25), Duration::from_millis(75)]
        );
        let source = (0..8193).map(|_| Ok(frame(10, 25)));
        let result = decode_animation_frames(
            Frames::new(Box::new(source)),
            1,
            1,
            &PreparationControl::new(false),
        );
        assert!(
            matches!(result, Err(MediaError::InvalidConfig(message)) if message.contains("8,192"))
        );
    }

    #[test]
    fn cancellation_is_checked_before_requesting_another_decoded_frame() {
        let control = PreparationControl::new(false);
        control.cancel();
        let source = std::iter::from_fn(|| -> Option<image::ImageResult<image::Frame>> {
            panic!("Cancelled preparation requested a frame")
        });
        assert!(matches!(
            decode_animation_frames(Frames::new(Box::new(source)), 1, 1, &control),
            Err(MediaError::Cancelled)
        ));
    }

    #[test]
    fn gif_and_png_sources_reject_oversized_geometry_before_resizing() {
        let directory = tempfile::tempdir().unwrap();
        for extension in ["gif", "png"] {
            let path = directory.path().join(format!("oversized.{extension}"));
            image::RgbImage::new(8193, 1).save(&path).unwrap();
            assert!(matches!(
                decode_frames_to_rgba(&path, 10.0, 1, 1, false),
                Err(MediaError::Image(image::ImageError::Limits(_)))
            ));
        }
    }

    #[test]
    fn jpeg_gif_preparation_rejects_excess_frames_instead_of_truncating() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("many.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        for _ in 0..8193 {
            encoder
                .encode(&[10, 20, 30], 1, 1, image::ExtendedColorType::Rgb8)
                .unwrap();
        }
        drop(encoder);
        let screen = ScreenInfo {
            width: 1,
            height: 1,
            ..ScreenInfo::WIRELESS_LCD
        };
        assert!(
            matches!(build_gif_frames(&path, 0.0, &screen, None), Err(MediaError::InvalidConfig(message)) if message.contains("8,192"))
        );
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use lianli_shared::template::ImageFit;
    use std::fs::File;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn two_tone() -> RgbaImage {
        RgbaImage::from_fn(32, 16, |x, _| {
            let [r, g, b] = if x < 16 { RED } else { BLUE };
            image::Rgba([r, g, b, 255])
        })
    }

    fn sources(directory: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let png = directory.join("source.png");
        two_tone().save(&png).unwrap();
        let gif = directory.join("source.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&gif).unwrap());
        encoder
            .encode_frame(image::Frame::from_parts(
                two_tone(),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
        drop(encoder);
        (png, gif)
    }

    fn frames(directory: &Path, framing: &MediaFraming) -> Vec<image::RgbImage> {
        let (png, gif) = sources(directory);
        let screen = ScreenInfo {
            width: 16,
            height: 16,
            ..ScreenInfo::WIRELESS_LCD
        };
        let control = PreparationControl::new(false);
        let image_frame = crate::image::load_image_frame(&png, 0.0, framing, &screen).unwrap();
        let (gif_frames, _) =
            build_gif_frames_cancellable(&gif, 0.0, &screen, None, framing, &control).unwrap();
        let (video_frames, _) =
            build_video_frames(&gif, 10.0, 0.0, framing, &screen, false).unwrap();
        [&image_frame, &gif_frames[0], &video_frames[0]]
            .into_iter()
            .map(|jpeg| image::load_from_memory(jpeg).unwrap().to_rgb8())
            .collect()
    }

    fn dominant(pixel: &image::Rgb<u8>) -> [u8; 3] {
        let [r, g, b] = pixel.0;
        if r < 40 && g < 40 && b < 40 {
            [0, 0, 0]
        } else if r > b {
            RED
        } else {
            BLUE
        }
    }

    fn cover(offset_x: f32) -> MediaFraming {
        MediaFraming {
            fit: ImageFit::Cover,
            offset_x,
            ..MediaFraming::default()
        }
    }

    #[test]
    fn every_lcd_path_pans_to_the_same_region() {
        let directory = tempfile::tempdir().unwrap();
        for (offset, expected) in [(-1.0, RED), (1.0, BLUE)] {
            for frame in frames(directory.path(), &cover(offset)) {
                assert_eq!(dominant(frame.get_pixel(8, 8)), expected, "offset {offset}");
            }
        }
    }

    #[test]
    fn contain_letterboxes_wide_sources_on_every_lcd_path() {
        let directory = tempfile::tempdir().unwrap();
        let contain = MediaFraming {
            fit: ImageFit::Contain,
            ..MediaFraming::default()
        };
        for frame in frames(directory.path(), &contain) {
            assert_eq!(dominant(frame.get_pixel(8, 0)), [0, 0, 0]);
            assert_eq!(dominant(frame.get_pixel(2, 8)), RED);
            assert_eq!(dominant(frame.get_pixel(13, 8)), BLUE);
        }
    }

    #[test]
    fn default_framing_keeps_the_existing_scale_filter() {
        assert_eq!(
            ffmpeg::framing_filter(&MediaFraming::default(), 400, 400),
            "scale=400:400:flags=lanczos"
        );
    }
}
