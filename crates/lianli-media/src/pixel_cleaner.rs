use image::{Rgb, RgbImage};

pub const FPS: u32 = 20;
pub const FRAME_COUNT: u64 = 100;

/// Fill a reusable buffer with one frame of the five-second loop.
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
    payload_limit: usize,
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
        || !(4096..=MAX_ASSET_BYTES).contains(&payload_limit)
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
    let mut encoding_screen = *screen;
    if screen.h264 {
        encoding_screen.width = (screen.width / 2).max(2) & !1;
        encoding_screen.height = (screen.height / 2).max(2) & !1;
        let noise_pixels = h264_budget(payload_limit).0 as f64 / f64::from(FPS) / 1.5;
        let scale = (noise_pixels
            / (f64::from(encoding_screen.width) * f64::from(encoding_screen.height)))
        .sqrt()
        .min(1.0);
        encoding_screen.width = ((f64::from(encoding_screen.width) * scale) as u32).max(2) & !1;
        encoding_screen.height = ((f64::from(encoding_screen.height) * scale) as u32).max(2) & !1;
    } else {
        encoding_screen.max_payload = payload_limit.min(screen.max_payload);
    }
    let (width, height) = crate::common::render_dimensions(&encoding_screen, orientation);
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
        let encoded = encode_frame(pixels, &encoding_screen)?;
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
        let (max_rate, buffer_bits) = h264_budget(payload_limit);
        tracing::info!(
            width = encoding_screen.width,
            height = encoding_screen.height,
            payload_limit,
            max_rate,
            buffer_bits,
            "Preparing pixel cleaner H.264"
        );
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
                "medium",
                "-profile:v",
                "baseline",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "20",
                "-bf",
                "0",
                "-maxrate",
                &max_rate.to_string(),
                "-bufsize",
                &buffer_bits.to_string(),
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
        validate_h264(&path, &encoding_screen, payload_limit, deadline, cancel)?;
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

fn h264_budget(payload_limit: usize) -> (usize, usize) {
    // Reserve headroom: quarter-block average frames, half-block bursts, and a decoder rate ceiling.
    let max_rate = (payload_limit as u64 * u64::from(FPS) * 8 / 4).min(16_000_000) as usize;
    let buffer_bits = (payload_limit * 8 / 2).min(max_rate / FPS as usize * 2);
    (max_rate, buffer_bits)
}

fn validate_h264(
    path: &std::path::Path,
    screen: &lianli_shared::screen::ScreenInfo,
    payload_limit: usize,
    deadline: std::time::Instant,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), crate::MediaError> {
    let mut command = std::process::Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_packets",
            "-show_entries",
            "packet=size:stream=width,height,nb_read_frames",
            "-of",
            "json",
        ])
        .arg(path);
    let output = crate::video::process::output_cancellable(
        command,
        deadline.saturating_duration_since(std::time::Instant::now()),
        cancel,
    )?;
    let valid = (|| -> Option<bool> {
        if !output.status.success() {
            return Some(false);
        }
        let data: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let stream = data.get("streams")?.get(0)?;
        let packets = data.get("packets")?.as_array()?;
        Some(
            stream.get("width")?.as_u64()? == u64::from(screen.width)
                && stream.get("height")?.as_u64()? == u64::from(screen.height)
                && stream.get("nb_read_frames")?.as_str()? == "100"
                && packets.len() == FRAME_COUNT as usize
                && packets.iter().all(|packet| {
                    packet
                        .get("size")
                        .and_then(|v| v.as_str())
                        .and_then(|v| v.parse::<usize>().ok())
                        .is_some_and(|size| size > 0 && size <= payload_limit)
                }),
        )
    })()
    .unwrap_or(false);
    if !valid {
        return Err(crate::MediaError::InvalidConfig(
            "generated H.264 exceeds the device payload limit or failed decode validation".into(),
        ));
    }
    Ok(())
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
    fn negotiated_h264_limits_change_encoding_budgets() {
        assert_eq!(h264_budget(65_536), (2_621_440, 262_144));
        assert_eq!(h264_budget(202_752), (8_110_080, 811_008));
        assert_eq!(h264_budget(1_048_576), (16_000_000, 1_600_000));
    }

    #[test]
    fn small_negotiated_blocks_produce_complete_validated_h264_loops() {
        let screen = lianli_shared::screen::ScreenInfo::UNIVERSAL_SCREEN;
        for payload_limit in [32_768, 65_536] {
            let asset = prepare_asset(
                &screen,
                90.0,
                payload_limit,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            assert!(matches!(
                asset,
                crate::MediaAssetKind::H264Stream { looping: true, .. }
            ));
        }
    }

    #[test]
    fn noise_fits_each_supported_screen_payload() {
        use lianli_shared::screen::ScreenInfo;
        for screen in [
            ScreenInfo::TLLCD,
            ScreenInfo::WIRELESS_LCD,
            ScreenInfo::AIO_LCD_480,
            ScreenInfo::HYDROSHIFT2,
            ScreenInfo::HYDROSHIFT2_OLED_CURVE,
            ScreenInfo::LANCOOL_207,
            ScreenInfo::UNIVERSAL_SCREEN,
            ScreenInfo::VISION_9P2,
            ScreenInfo::FLEX_LCD,
        ] {
            let mut frame = RgbImage::new(screen.width, screen.height);
            render_frame(&mut frame, 30);
            let encoded = encode_frame(&frame, &screen).unwrap();
            assert!(encoded.len() <= screen.max_payload);
        }
    }

    #[test]
    fn rotated_rectangular_loops_keep_native_output_dimensions() {
        let mut screen = lianli_shared::screen::ScreenInfo::TLLCD;
        screen.width = 32;
        screen.height = 64;
        for orientation in [0.0, 90.0, 180.0, 270.0] {
            let asset = prepare_asset(
                &screen,
                orientation,
                screen.max_payload,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let crate::MediaAssetKind::Video { frames, .. } = asset else {
                panic!("expected JPEG")
            };
            let frame = image::load_from_memory(&frames[30]).unwrap();
            assert_eq!((frame.width(), frame.height()), (32, 64));
        }
    }

    #[test]
    fn prepared_jpeg_loops_respect_wired_and_wireless_payloads_and_timing() {
        for screen in [
            lianli_shared::screen::ScreenInfo::TLLCD,
            lianli_shared::screen::ScreenInfo::WIRELESS_LCD,
        ] {
            let asset = prepare_asset(
                &screen,
                90.0,
                screen.max_payload,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
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
                assert_eq!(
                    (decoded.width(), decoded.height()),
                    (screen.width, screen.height)
                );
            }
        }
    }

    #[test]
    fn prepared_h264_loops_bound_bursts_on_every_supported_h264_screen() {
        use lianli_shared::screen::ScreenInfo;
        for screen in [
            ScreenInfo::AIO_LCD_480,
            ScreenInfo::HYDROSHIFT2,
            ScreenInfo::HYDROSHIFT2_OLED_CURVE,
            ScreenInfo::UNIVERSAL_SCREEN,
            ScreenInfo::FLEX_LCD,
        ] {
            let asset = prepare_asset(
                &screen,
                0.0,
                screen.max_payload.min(202_752),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
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
                    "-show_packets",
                    "-show_entries",
                    "packet=size:stream=width,height,nb_read_frames",
                    "-of",
                    "json",
                ])
                .arg(path);
            let result = crate::video::process::output_with_timeout(
                command,
                std::time::Duration::from_secs(10),
            )
            .unwrap();
            assert!(result.status.success());
            let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let width = data["streams"][0]["width"].as_u64().unwrap();
            let height = data["streams"][0]["height"].as_u64().unwrap();
            assert!(width <= u64::from(screen.width / 2) && height <= u64::from(screen.height / 2));
            let aspect_error = (width as f64 / height as f64)
                / (f64::from(screen.width) / f64::from(screen.height))
                - 1.0;
            assert!(aspect_error.abs() < 0.01);
            if screen == ScreenInfo::UNIVERSAL_SCREEN {
                assert_eq!((width, height), (240, 960));
            }
            assert_eq!(data["streams"][0]["nb_read_frames"], "100");
            let sizes: Vec<usize> = data["packets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|packet| packet["size"].as_str().unwrap().parse().unwrap())
                .collect();
            assert_eq!(sizes.len(), FRAME_COUNT as usize);
            assert!(
                sizes
                    .iter()
                    .all(|&size| size <= screen.max_payload.min(202_752)),
                "oversized H.264 frame on {screen:?}: {sizes:?}"
            );
            assert!(
                sizes
                    .windows(FPS as usize)
                    .all(|second| second.iter().sum::<usize>()
                        <= (h264_budget(screen.max_payload.min(202_752)).0
                            + h264_budget(screen.max_payload.min(202_752)).1)
                            / 8),
                "H.264 bitrate burst on {screen:?}"
            );

            {
                let mut command = std::process::Command::new("ffmpeg");
                command.args(["-v", "error", "-i"]).arg(path).args([
                    "-vf",
                    "select=eq(n\\,35)",
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "-",
                ]);
                let frame = crate::video::process::output_with_timeout(
                    command,
                    std::time::Duration::from_secs(10),
                )
                .unwrap();
                assert!(frame.status.success());
                let pixels = (width * height) as usize;
                assert_eq!(frame.stdout.len(), pixels * 3);
                let dark = frame
                    .stdout
                    .iter()
                    .step_by(3)
                    .filter(|&&red| red < 48)
                    .count();
                let bright = frame
                    .stdout
                    .iter()
                    .step_by(3)
                    .filter(|&&red| red > 207)
                    .count();
                assert!(dark > pixels * 10 / 100 && bright > pixels * 10 / 100,
                    "noise contrast lost on {screen:?}: dark={dark}, bright={bright}, pixels={pixels}");
            }
            let path = path.clone();
            drop(asset);
            assert!(!path.exists());
        }
    }

    #[test]
    fn cancelled_preparation_and_unsupported_frame_rate_are_rejected() {
        let mut screen = lianli_shared::screen::ScreenInfo::TLLCD;
        assert!(prepare_asset(
            &screen,
            0.0,
            screen.max_payload.min(202_752),
            &std::sync::atomic::AtomicBool::new(true)
        )
        .is_err());
        screen.max_fps = 10;
        assert!(prepare_asset(
            &screen,
            0.0,
            screen.max_payload.min(202_752),
            &std::sync::atomic::AtomicBool::new(false)
        )
        .is_err());
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
