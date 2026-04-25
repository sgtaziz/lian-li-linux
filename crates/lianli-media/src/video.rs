use super::common::{apply_orientation, encode_jpeg, render_dimensions, MediaError};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::imageops::FilterType;
use image::{load_from_memory, AnimationDecoder, DynamicImage, Frames, RgbaImage};
use lianli_shared::screen::ScreenInfo;
use std::fs::File;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tracing::{debug, info};

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
        if orientation.abs() < f32::EPSILON && screen.device_rotation == 0 {
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
    let file = File::open(path)?;
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
        let decoder = GifDecoder::new(File::open(path)?)?;
        return decode_animation_frames(decoder.into_frames(), width, height);
    }

    if ext == "png" || ext == "apng" {
        let decoder = PngDecoder::new(File::open(path)?)?;
        if decoder.is_apng() {
            let apng = decoder.apng();
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

pub fn encode_h264(
    input: &Path,
    fps: f32,
    orientation: f32,
    screen: &ScreenInfo,
) -> Result<(std::path::PathBuf, tempfile::TempDir), MediaError> {
    let temp = TempDir::new()?;
    let output = temp.path().join("stream.h264");

    let (rw, rh) = render_dimensions(screen, orientation);
    let mut vf_parts = vec![format!("scale={rw}:{rh}:flags=lanczos")];
    let rot = (orientation % 360.0 + 360.0) % 360.0;
    if (rot - 90.0).abs() < 1.0 {
        vf_parts.push("transpose=1".into());
    } else if (rot - 180.0).abs() < 1.0 {
        vf_parts.push("transpose=1,transpose=1".into());
    } else if (rot - 270.0).abs() < 1.0 {
        vf_parts.push("transpose=2".into());
    }
    let vf = vf_parts.join(",");

    let fps_int = fps.round().max(1.0) as u32;
    let (out_w, out_h) = target_dimensions(screen, orientation);
    let bitrate = (out_w as u64 * out_h as u64 * fps_int as u64 / 4).max(1_000_000);
    let bitrate_str = format!("{bitrate}");
    let fps_str = fps_int.to_string();

    let encoders: &[EncoderKind] = if hw_video_disabled() {
        &[EncoderKind::Libx264]
    } else {
        &[
            EncoderKind::Nvenc,
            EncoderKind::Amf,
            EncoderKind::Vaapi,
            EncoderKind::Qsv,
            EncoderKind::Libx264,
        ]
    };

    let mut last_stderr: Option<String> = None;
    for kind in encoders {
        match run_encode(input, &vf, &fps_str, &bitrate_str, *kind, &output) {
            Ok(()) => {
                info!(
                    "LCD H.264 transcode: {out_w}x{out_h}@{fps_int}fps via {}",
                    kind.name()
                );
                return Ok((output, temp));
            }
            Err(stderr) => {
                debug!("LCD H.264 encoder {} unavailable: {stderr}", kind.name());
                last_stderr = Some(stderr);
            }
        }
    }

    Err(MediaError::Ffmpeg(format!(
        "all H.264 encoders failed; last error: {}",
        last_stderr.unwrap_or_default()
    )))
}

#[derive(Debug, Clone, Copy)]
enum EncoderKind {
    Nvenc,
    Amf,
    Vaapi,
    Qsv,
    Libx264,
}

impl EncoderKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Amf => "h264_amf",
            Self::Vaapi => "h264_vaapi",
            Self::Qsv => "h264_qsv",
            Self::Libx264 => "libx264",
        }
    }
}

fn hw_video_disabled() -> bool {
    std::env::var("LIANLI_DISABLE_HW_VIDEO")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

fn run_encode(
    input: &Path,
    vf: &str,
    fps_str: &str,
    bitrate_str: &str,
    kind: EncoderKind,
    output: &Path,
) -> Result<(), String> {
    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];

    // Pre-input device init / hwaccel flags (encoder-specific).
    match kind {
        EncoderKind::Vaapi => {
            args.extend([
                "-vaapi_device".into(),
                "/dev/dri/renderD128".into(),
                "-hwaccel".into(),
                "vaapi".into(),
            ]);
        }
        EncoderKind::Qsv => {
            args.extend([
                "-init_hw_device".into(),
                "qsv=qsv".into(),
                "-filter_hw_device".into(),
                "qsv".into(),
                "-hwaccel".into(),
                "qsv".into(),
            ]);
        }
        _ => {
            if !hw_video_disabled() {
                args.extend(["-hwaccel".into(), "auto".into()]);
            }
        }
    }

    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);

    // VAAPI/QSV need the filter chain to end by uploading NV12 frames to GPU surfaces.
    let vf_final: String = match kind {
        EncoderKind::Vaapi => format!("{vf},format=nv12,hwupload"),
        EncoderKind::Qsv => format!("{vf},format=nv12,hwupload=extra_hw_frames=16"),
        _ => vf.to_string(),
    };
    args.extend(["-vf".into(), vf_final]);
    args.extend(["-r".into(), fps_str.into()]);
    args.extend(["-c:v".into(), kind.name().into()]);
    args.extend(["-b:v".into(), bitrate_str.into()]);

    match kind {
        EncoderKind::Nvenc => {
            args.extend(["-preset".into(), "p1".into()]);
            args.extend(["-tune".into(), "ll".into()]);
            args.extend(["-rc".into(), "vbr".into()]);
        }
        EncoderKind::Amf => {
            args.extend(["-usage".into(), "lowlatency".into()]);
            args.extend(["-quality".into(), "speed".into()]);
        }
        EncoderKind::Vaapi => {
            args.extend(["-rc_mode".into(), "VBR".into()]);
        }
        EncoderKind::Qsv => {
            args.extend(["-preset".into(), "veryfast".into()]);
            args.extend(["-look_ahead".into(), "0".into()]);
        }
        EncoderKind::Libx264 => {
            args.extend(["-preset".into(), "ultrafast".into()]);
            args.extend(["-x264-params".into(), "bframes=0:no-scenecut=1".into()]);
        }
    }

    // -pix_fmt only applies to the software-output encoders; VAAPI/QSV write
    // GPU surfaces described by the filter chain above.
    if !matches!(kind, EncoderKind::Vaapi | EncoderKind::Qsv) {
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    }

    args.extend([
        "-an".into(),
        "-t".into(),
        "30".into(),
        "-f".into(),
        "h264".into(),
        output.to_string_lossy().into_owned(),
    ]);

    let output_result = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;
    if output_result.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        Err(stderr.trim().to_string())
    }
}

fn target_dimensions(screen: &ScreenInfo, orientation: f32) -> (u32, u32) {
    let (rw, rh) = render_dimensions(screen, orientation);
    let rot = (orientation % 360.0 + 360.0) % 360.0;
    if (rot - 90.0).abs() < 1.0 || (rot - 270.0).abs() < 1.0 {
        (rh, rw)
    } else {
        (rw, rh)
    }
}

fn run_ffmpeg(
    input: &Path,
    fps: f32,
    output_pattern: &Path,
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    let scale_filter = format!("scale={width}:{height}:flags=lanczos");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            input.to_str().unwrap(),
            "-vf",
            &scale_filter,
            "-r",
            &fps.to_string(),
            "-q:v",
            "4",
            output_pattern.to_str().unwrap(),
        ])
        .status()
        .map_err(MediaError::Io)?;

    if !status.success() {
        return Err(MediaError::Ffmpeg(format!(
            "ffmpeg exited with status {status}"
        )));
    }

    Ok(())
}

fn run_ffmpeg_rgba(
    input: &Path,
    fps: f32,
    output_pattern: &Path,
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    let scale_filter = format!("scale={width}:{height}:flags=lanczos");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            input.to_str().unwrap(),
            "-vf",
            &scale_filter,
            "-r",
            &fps.to_string(),
            "-pix_fmt",
            "rgba",
            output_pattern.to_str().unwrap(),
        ])
        .status()
        .map_err(MediaError::Io)?;

    if !status.success() {
        return Err(MediaError::Ffmpeg(format!(
            "ffmpeg exited with status {status}"
        )));
    }

    Ok(())
}
