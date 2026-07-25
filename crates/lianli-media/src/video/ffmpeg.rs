use crate::common::MediaError;
use std::path::Path;
use std::process::Command;

/// Probe the source's average frame rate via ffprobe. Returns `None` if the
/// file isn't a video/animated source or ffprobe is unavailable.
pub fn probe_source_fps(path: &Path) -> Option<f32> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let s = s.trim();
    let (num, den) = s.split_once('/')?;
    let num: f32 = num.parse().ok()?;
    let den: f32 = den.parse().ok()?;
    if den <= 0.0 || num <= 0.0 {
        return None;
    }
    Some(num / den)
}

/// Cap `target` by the source's native fps (when probeable). Always at least 1.
pub fn cap_fps_to_source(path: &Path, target: f32) -> f32 {
    let target = target.max(1.0);
    match probe_source_fps(path) {
        Some(src) if src >= 1.0 => target.min(src),
        _ => target,
    }
}

fn hwaccel_args() -> Vec<String> {
    if super::h264::hw_video_enabled() {
        vec!["-hwaccel".into(), "auto".into()]
    } else {
        Vec::new()
    }
}

pub(super) fn run_ffmpeg(
    input: &Path,
    fps: f32,
    output_pattern: &Path,
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    let scale_filter = format!("scale={width}:{height}:flags=lanczos");
    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];
    args.extend(hwaccel_args());
    args.extend([
        "-i".into(),
        input.to_str().unwrap().into(),
        "-vf".into(),
        scale_filter,
        "-r".into(),
        fps.to_string(),
        "-q:v".into(),
        "4".into(),
        output_pattern.to_str().unwrap().into(),
    ]);
    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(MediaError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::Ffmpeg(format!(
            "ffmpeg exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    Ok(())
}

pub(super) fn run_ffmpeg_rgba(
    input: &Path,
    fps: f32,
    output_pattern: &Path,
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    let scale_filter = format!("scale={width}:{height}:flags=lanczos");
    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];
    args.extend(hwaccel_args());
    args.extend([
        "-i".into(),
        input.to_str().unwrap().into(),
        "-vf".into(),
        scale_filter,
        "-r".into(),
        fps.to_string(),
        "-pix_fmt".into(),
        "rgba".into(),
        output_pattern.to_str().unwrap().into(),
    ]);
    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(MediaError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::Ffmpeg(format!(
            "ffmpeg exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    Ok(())
}
