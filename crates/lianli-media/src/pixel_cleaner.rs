use image::{Rgb, RgbImage};

pub const FPS: u32 = 20;
pub const FRAME_COUNT: u64 = 100;

/// Fill a reusable, native-size buffer with one frame of the five-second loop.
pub fn render_frame(frame: &mut RgbImage, frame_index: u64) {
    let frame_index = frame_index % FRAME_COUNT;
    let color = match frame_index {
        0..30 => Some(if frame_index.is_multiple_of(2) {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        }),
        30..60 => None,
        60..68 => Some([255, 0, 0]),
        68..76 => Some([0, 128, 0]),
        76..84 => Some([0, 0, 255]),
        84..92 => Some([255, 255, 255]),
        _ => Some([128, 128, 128]),
    };

    if let Some(color) = color {
        for pixel in frame.pixels_mut() {
            *pixel = Rgb(color);
        }
        return;
    }

    let mut random = 0x9e37_79b9_u32 ^ frame_index as u32;
    for pixel in frame.pixels_mut() {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        let luma = (random >> 24) as i32;
        // Match the reference noise's contrast after limited-range luma expansion.
        let gray = ((luma - 16) * 255 / 219).clamp(0, 255) as u8;
        *pixel = Rgb([gray; 3]);
    }
}

pub const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;

pub fn prepare_asset(
    screen: &lianli_shared::screen::ScreenInfo,
    orientation: f32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<crate::MediaAssetKind, crate::MediaError> {
    use crate::MediaAssetKind;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    if screen.width == 0
        || screen.height == 0
        || u64::from(screen.width) * u64::from(screen.height) > 4096 * 4096
        || screen.max_fps < FPS
        || !orientation.is_finite()
    {
        return Err(crate::MediaError::InvalidConfig(
            "unsupported pixel cleaner screen or orientation".into(),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let cancelled = || {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            Err(crate::MediaError::InvalidConfig(
                "pixel cleaner preparation cancelled or timed out".into(),
            ))
        } else {
            Ok(())
        }
    };
    cancelled()?;
    let (width, height) = crate::common::render_dimensions(screen, orientation);
    let mut frame = RgbImage::new(width, height);
    let frames_dir = if screen.h264 {
        Some(tempfile::TempDir::new()?)
    } else {
        None
    };
    let mut frames = Vec::new();
    let mut bytes = 0;
    for index in 0..FRAME_COUNT {
        cancelled()?;
        render_frame(&mut frame, index);
        let oriented;
        let pixels = if orientation.rem_euclid(360.0).abs() < f32::EPSILON {
            &frame
        } else {
            oriented = crate::common::apply_orientation(frame.clone(), orientation);
            &oriented
        };
        let encoded = encode_frame(pixels, screen)?;
        bytes += encoded.len();
        if bytes > MAX_ASSET_BYTES {
            return Err(crate::MediaError::InvalidConfig(
                "pixel cleaner asset exceeds memory budget".into(),
            ));
        }
        if let Some(dir) = &frames_dir {
            std::fs::write(dir.path().join(format!("frame_{index:03}.jpg")), encoded)?;
        } else {
            frames.push(encoded);
        }
    }
    cancelled()?;
    if let Some(frames_dir) = frames_dir {
        let output_dir = tempfile::TempDir::new()?;
        let path = output_dir.path().join("cleaner.h264");
        let mut command = std::process::Command::new("ffmpeg");
        command
            .args(["-y", "-loglevel", "error", "-framerate", "20", "-i"])
            .arg(frames_dir.path().join("frame_%03d.jpg"))
            .args([
                "-frames:v",
                "100",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "20",
                "-bf",
                "0",
                "-x264-params",
                "repeat-headers=1:aud=1",
                "-fs",
                "33554432",
                "-an",
                "-f",
                "h264",
            ])
            .arg(&path);
        let output = crate::video::process::output_cancellable(
            command,
            deadline.saturating_duration_since(Instant::now()),
            cancel,
        )?;
        if !output.status.success() {
            return Err(crate::MediaError::Ffmpeg(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let size = std::fs::metadata(&path)?.len();
        if size == 0 || size >= MAX_ASSET_BYTES as u64 {
            return Err(crate::MediaError::InvalidConfig(
                "generated H.264 asset is empty or exceeds its size limit".into(),
            ));
        }
        cancelled()?;
        Ok(MediaAssetKind::H264Stream {
            path,
            looping: true,
            fps: FPS as f32,
            _temp: Arc::new(output_dir),
        })
    } else {
        Ok(MediaAssetKind::Video {
            frame_durations: Arc::new(vec![Duration::from_millis(50); frames.len()]),
            frames: Arc::new(frames),
        })
    }
}

fn encode_frame(
    frame: &RgbImage,
    screen: &lianli_shared::screen::ScreenInfo,
) -> Result<Vec<u8>, crate::MediaError> {
    let mut encoding = *screen;
    // H.264 preparation uses temporary JPEG inputs, even on PNG-overlay panels.
    encoding.png = screen.png && !screen.h264;
    let mut last_error = None;
    for quality in [screen.jpeg_quality, 80, 65, 50, 35, 20] {
        encoding.jpeg_quality = quality.min(screen.jpeg_quality);
        let image = turbojpeg::Image {
            pixels: frame.as_raw().as_slice(),
            width: frame.width() as usize,
            pitch: frame.width() as usize * 3,
            height: frame.height() as usize,
            format: turbojpeg::PixelFormat::RGB,
        };
        match crate::common::encode_compressed(image, &encoding) {
            Ok(bytes) => return Ok(bytes),
            Err(e @ crate::MediaError::PayloadTooLarge { .. }) if !encoding.png => {
                last_error = Some(e)
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_error.expect("all quality attempts exceeded the payload limit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_jpeg_loop_respects_tl_payloads_and_timing() {
        let screen = lianli_shared::screen::ScreenInfo::TLLCD;
        let asset =
            prepare_asset(&screen, 90.0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let crate::MediaAssetKind::Video {
            frames,
            frame_durations,
        } = asset
        else {
            panic!("expected JPEG frames")
        };
        assert_eq!(frames.len(), 100);
        assert_eq!(
            frame_durations.iter().sum::<std::time::Duration>(),
            std::time::Duration::from_secs(5)
        );
        for frame in frames.iter() {
            assert!(frame.len() <= screen.max_payload);
            let decoded = image::load_from_memory(frame).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (400, 400));
        }
    }

    #[test]
    fn prepared_h264_loop_decodes_all_frames_and_removes_temporary_file() {
        let screen = lianli_shared::screen::ScreenInfo::AIO_LCD_480;
        let asset =
            prepare_asset(&screen, 0.0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let crate::MediaAssetKind::H264Stream {
            ref path,
            fps,
            looping,
            ..
        } = asset
        else {
            panic!("expected H.264")
        };
        assert_eq!(fps, 20.0);
        assert!(looping);
        let mut command = std::process::Command::new("ffprobe");
        command
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_entries",
                "stream=width,height,nb_read_frames",
                "-of",
                "json",
            ])
            .arg(path);
        let result =
            crate::video::process::output_with_timeout(command, std::time::Duration::from_secs(10))
                .unwrap();
        assert!(result.status.success());
        let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(data["streams"][0]["width"], 480);
        assert_eq!(data["streams"][0]["height"], 480);
        assert_eq!(data["streams"][0]["nb_read_frames"], "100");
        let path = path.clone();
        drop(asset);
        assert!(!path.exists());
    }

    #[test]
    fn cancelled_preparation_and_unsupported_frame_rate_are_rejected() {
        let mut screen = lianli_shared::screen::ScreenInfo::TLLCD;
        assert!(prepare_asset(&screen, 0.0, &std::sync::atomic::AtomicBool::new(true)).is_err());
        screen.max_fps = 10;
        assert!(prepare_asset(&screen, 0.0, &std::sync::atomic::AtomicBool::new(false)).is_err());
    }

    #[test]
    fn solid_phases_cover_the_frame_at_the_reference_times() {
        let mut frame = RgbImage::new(13, 29);
        for index in 0..FRAME_COUNT {
            let expected = match index {
                0..30 if index % 2 == 0 => [0, 0, 0],
                0..30 => [255, 255, 255],
                30..60 => continue,
                60..68 => [255, 0, 0],
                68..76 => [0, 128, 0],
                76..84 => [0, 0, 255],
                84..92 => [255, 255, 255],
                _ => [128, 128, 128],
            };
            render_frame(&mut frame, index);
            assert!(frame.pixels().all(|pixel| pixel.0 == expected));
        }
        assert_eq!(FRAME_COUNT * 1000 / u64::from(FPS), 5000);
    }

    #[test]
    fn noise_changes_between_frames_and_repeats_after_one_loop() {
        let mut first = RgbImage::new(97, 53);
        let mut next = first.clone();
        render_frame(&mut first, 30);
        render_frame(&mut next, 31);
        assert_ne!(first, next);
        render_frame(&mut next, 130);
        assert_eq!(first, next);
        assert!(first.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        assert!(first.pixels().any(|p| p[0] == 0));
        assert!(first.pixels().any(|p| p[0] == 255));
        let mean = first.pixels().map(|p| f64::from(p[0])).sum::<f64>()
            / f64::from(first.width() * first.height());
        assert!((120.0..136.0).contains(&mean));
    }

    #[test]
    fn rendering_reuses_the_buffer_and_wraps_at_the_loop_boundary() {
        let mut frame = RgbImage::new(23, 11);
        let pixels = frame.as_ptr();
        render_frame(&mut frame, 99);
        assert!(frame.pixels().all(|p| p.0 == [128; 3]));
        render_frame(&mut frame, 100);
        assert!(frame.pixels().all(|p| p.0 == [0; 3]));
        assert_eq!(frame.dimensions(), (23, 11));
        assert_eq!(frame.as_ptr(), pixels);
    }
}
