use super::target_dimensions;
use crate::common::{render_dimensions, MediaError};
use lianli_shared::screen::ScreenInfo;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use tracing::{debug, info};

pub fn encode_h264(
    input: &Path,
    fps: f32,
    orientation: f32,
    screen: &ScreenInfo,
) -> Result<(PathBuf, TempDir, f32), MediaError> {
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

    let mut last_stderr: Option<String> = None;
    for kind in encoder_chain() {
        match run_encode(input, &vf, &fps_str, &bitrate_str, *kind, &output) {
            Ok(()) => {
                info!(
                    "LCD H.264 transcode: {out_w}x{out_h}@{fps_int}fps via {}",
                    kind.name()
                );
                return Ok((output, temp, fps_int as f32));
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
pub(super) enum EncoderKind {
    Nvenc,
    Amf,
    Vaapi,
    Vulkan,
    Qsv,
    Libx264,
}

impl EncoderKind {
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Amf => "h264_amf",
            Self::Vaapi => "h264_vaapi",
            Self::Vulkan => "h264_vulkan",
            Self::Qsv => "h264_qsv",
            Self::Libx264 => "libx264",
        }
    }
}

pub(super) fn hw_video_enabled() -> bool {
    !std::env::var("LIANLI_DISABLE_HW_VIDEO")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(super) fn encoder_chain() -> &'static [EncoderKind] {
    if hw_video_enabled() {
        &[
            EncoderKind::Nvenc,
            EncoderKind::Amf,
            EncoderKind::Vaapi,
            EncoderKind::Libx264,
            EncoderKind::Vulkan,
            EncoderKind::Qsv,
        ]
    } else {
        &[EncoderKind::Libx264]
    }
}

pub(super) fn hwaccel_input_args(kind: EncoderKind) -> Vec<String> {
    match kind {
        EncoderKind::Vaapi => vec!["-vaapi_device".into(), "/dev/dri/renderD128".into()],
        EncoderKind::Vulkan => vec![
            "-init_hw_device".into(),
            "vulkan=vk".into(),
            "-filter_hw_device".into(),
            "vk".into(),
        ],
        EncoderKind::Qsv => vec![
            "-init_hw_device".into(),
            "qsv=qsv".into(),
            "-filter_hw_device".into(),
            "qsv".into(),
        ],
        _ => Vec::new(),
    }
}

/// Append the hwupload suffix to a -vf chain when the encoder needs frames on GPU surfaces.
pub(super) fn finalize_vf(kind: EncoderKind, vf: &str) -> String {
    match kind {
        EncoderKind::Vaapi => {
            if vf.is_empty() {
                "format=nv12,hwupload".into()
            } else {
                format!("{vf},format=nv12,hwupload")
            }
        }
        EncoderKind::Vulkan => {
            if vf.is_empty() {
                "format=nv12,hwupload".into()
            } else {
                format!("{vf},format=nv12,hwupload")
            }
        }
        EncoderKind::Qsv => {
            if vf.is_empty() {
                "format=nv12,hwupload=extra_hw_frames=16".into()
            } else {
                format!("{vf},format=nv12,hwupload=extra_hw_frames=16")
            }
        }
        _ => vf.to_string(),
    }
}

pub(super) fn encoder_codec_args_file(
    kind: EncoderKind,
    fps_str: &str,
    bitrate_str: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-r".into(),
        fps_str.into(),
        "-c:v".into(),
        kind.name().into(),
    ];

    match kind {
        EncoderKind::Libx264 => {
            args.extend(["-preset".into(), "ultrafast".into()]);
            args.extend(["-x264opts".into(), "bframes=0".into()]);
            args.extend(["-threads".into(), "4".into()]);
        }
        EncoderKind::Nvenc => {
            args.extend(["-preset".into(), "p1".into()]);
            args.extend(["-rc".into(), "vbr".into()]);
            args.extend(["-forced-idr".into(), "1".into()]);
            args.extend(["-b:v".into(), bitrate_str.into()]);
        }
        EncoderKind::Amf => {
            args.extend(["-usage".into(), "lowlatency".into()]);
            args.extend(["-quality".into(), "speed".into()]);
            args.extend(["-b:v".into(), bitrate_str.into()]);
        }
        EncoderKind::Vaapi => {
            args.extend(["-rc_mode".into(), "VBR".into()]);
            args.extend(["-bf".into(), "0".into()]);
            args.extend(["-b:v".into(), bitrate_str.into()]);
        }
        EncoderKind::Vulkan => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-bf".into(), "0".into()]);
        }
        EncoderKind::Qsv => {
            args.extend(["-preset".into(), "veryfast".into()]);
            args.extend(["-look_ahead".into(), "0".into()]);
            args.extend(["-bf".into(), "0".into()]);
            args.extend(["-b:v".into(), bitrate_str.into()]);
        }
    }

    if !matches!(
        kind,
        EncoderKind::Vaapi | EncoderKind::Vulkan | EncoderKind::Qsv
    ) {
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    }

    args
}

pub(super) fn encoder_codec_args_live(
    kind: EncoderKind,
    fps_str: &str,
    bitrate_str: &str,
) -> Vec<String> {
    let gop = fps_str.parse::<u32>().unwrap_or(30).max(1).to_string();
    let mut args: Vec<String> = vec![
        "-r".into(),
        fps_str.into(),
        "-g".into(),
        gop.clone(),
        "-keyint_min".into(),
        gop,
        "-c:v".into(),
        kind.name().into(),
    ];

    match kind {
        EncoderKind::Nvenc => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-preset".into(), "p1".into()]);
            args.extend(["-rc".into(), "vbr".into()]);
            args.extend(["-forced-idr".into(), "1".into()]);
        }
        EncoderKind::Amf => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-usage".into(), "lowlatency".into()]);
            args.extend(["-quality".into(), "speed".into()]);
        }
        EncoderKind::Vaapi => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-rc_mode".into(), "VBR".into()]);
            args.extend(["-bf".into(), "0".into()]);
        }
        EncoderKind::Vulkan => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-bf".into(), "0".into()]);
        }
        EncoderKind::Qsv => {
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-preset".into(), "veryfast".into()]);
            args.extend(["-look_ahead".into(), "0".into()]);
            args.extend(["-bf".into(), "0".into()]);
        }
        EncoderKind::Libx264 => {
            args.extend(["-preset".into(), "ultrafast".into()]);
            args.extend(["-tune".into(), "zerolatency".into()]);
            args.extend(["-b:v".into(), bitrate_str.into()]);
            args.extend(["-x264-params".into(), "bframes=0".into()]);
            args.extend(["-threads".into(), "4".into()]);
        }
    }

    if !matches!(
        kind,
        EncoderKind::Vaapi | EncoderKind::Vulkan | EncoderKind::Qsv
    ) {
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    }

    args
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
    args.extend(hwaccel_input_args(kind));
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    args.extend(["-vf".into(), finalize_vf(kind, vf)]);
    args.extend(encoder_codec_args_file(kind, fps_str, bitrate_str));
    args.extend([
        "-an".into(),
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
