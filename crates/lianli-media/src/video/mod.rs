pub mod ffmpeg;
pub mod h264;
pub mod h264_live;

pub use h264::encode_h264;
pub use h264_live::LiveH264Encoder;

use crate::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use ffmpeg::{run_ffmpeg, run_ffmpeg_rgba};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::imageops::FilterType;
use image::{load_from_memory, AnimationDecoder, DynamicImage, Frames, RgbaImage};
use lianli_shared::screen::ScreenInfo;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

pub fn build_video_frames(
    path: &Path,
    fps: f32,
    orientation: f32,
    screen: &ScreenInfo,
) -> Result<(Vec<Vec<u8>>, Vec<Duration>), MediaError> {
    let temp = TempDir::new()?;
    let output_pattern = temp.path().join("frame_%05d.jpg");
    let (rw, rh) = render_dimensions(screen, orientation);
    run_ffmpeg(path, fps, &output_pattern, rw, rh)?;

    let mut entries: Vec<_> = std::fs::read_dir(temp.path())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.extension().map(|ext| ext == "jpg").unwrap_or(false))
        .collect();
    entries.sort();

    if entries.is_empty() {
        return Err(MediaError::EmptyVideo);
    }

    let mut frames = Vec::with_capacity(entries.len());
    for frame_path in entries {
        let data = std::fs::read(&frame_path)?;
        if orientation.abs() < f32::EPSILON {
            if data.len() > screen.max_payload {
                return Err(MediaError::PayloadTooLarge { size: data.len() });
            }
            frames.push(data);
        } else {
            let image = load_from_memory(&data)?;
            let rgb = apply_orientation(image.to_rgb8(), orientation);
            frames.push(encode_jpeg(rgb, screen)?);
        }
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
    let file = BufReader::new(File::open(path)?);
    let decoder = GifDecoder::new(file)?;
    let mut encoded = Vec::new();
    let mut durations = Vec::new();

    let target_ms = desired_fps.map(|fps| 1000.0 / fps.max(1.0));
    let mut accum_ms = 0.0f32;

    let frames: Vec<_> = decoder
        .into_frames()
        .collect::<Result<Vec<_>, _>>()
        .map_err(image::ImageError::from)?;
    let n = frames.len();

    for (i, frame) in frames.into_iter().enumerate() {
        let (numer, denom) = frame.delay().numer_denom_ms();
        let native_ms = if denom == 0 {
            numer as f32
        } else {
            numer as f32 / denom as f32
        };
        let native_ms = native_ms.max(10.0);
        accum_ms += native_ms;

        let is_last = i + 1 == n;
        let should_emit = match target_ms {
            Some(t) => accum_ms >= t || is_last,
            None => true,
        };
        if !should_emit {
            continue;
        }

        let rgba = frame.into_buffer();
        let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
        let (rw, rh) = render_dimensions(screen, orientation);
        let resized = image::imageops::resize(&rgb, rw, rh, FilterType::Lanczos3);
        let oriented = apply_orientation(resized, orientation);
        let jpeg = encode_jpeg(oriented, screen)?;
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
) -> Result<(Vec<RgbaImage>, Vec<Duration>), MediaError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "gif" {
        let decoder = GifDecoder::new(BufReader::new(File::open(path)?))?;
        return decode_animation_frames(decoder.into_frames(), width, height);
    }

    if ext == "png" || ext == "apng" {
        let decoder = PngDecoder::new(BufReader::new(File::open(path)?))?;
        if decoder.is_apng()? {
            let apng = decoder.apng()?;
            return decode_animation_frames(apng.into_frames(), width, height);
        }
        let img = DynamicImage::from_decoder(decoder)?;
        let resized = image::imageops::resize(&img.to_rgba8(), width, height, FilterType::Lanczos3);
        return Ok((vec![resized], vec![Duration::from_millis(100)]));
    }

    let temp = TempDir::new()?;
    let output_pattern = temp.path().join("frame_%05d.png");
    run_ffmpeg_rgba(path, fps, &output_pattern, width, height)?;

    let mut entries: Vec<_> = std::fs::read_dir(temp.path())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "png").unwrap_or(false))
        .collect();
    entries.sort();

    if entries.is_empty() {
        return Err(MediaError::EmptyVideo);
    }

    let mut frames = Vec::with_capacity(entries.len());
    for frame_path in entries {
        let data = std::fs::read(&frame_path)?;
        let img = load_from_memory(&data)?;
        frames.push(img.to_rgba8());
    }

    let interval = Duration::from_secs_f32(1.0 / fps.max(1.0));
    let durations = vec![interval; frames.len()];
    Ok((frames, durations))
}

fn decode_animation_frames(
    frames: Frames<'_>,
    width: u32,
    height: u32,
) -> Result<(Vec<RgbaImage>, Vec<Duration>), MediaError> {
    let mut out_frames = Vec::new();
    let mut durations = Vec::new();
    for frame in frames {
        let frame = frame?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let millis = if denom == 0 {
            numer as f32
        } else {
            numer as f32 / denom as f32
        };
        let duration = Duration::from_millis(millis.max(10.0) as u64);
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
