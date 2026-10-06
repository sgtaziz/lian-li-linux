use super::process::ENCODE_TIMEOUT;
use super::target_dimensions;
use crate::common::{render_dimensions, MediaError};
use crate::PreparationControl;
use crate::TemporaryMedia;
use lianli_shared::media::MediaFraming;
use lianli_shared::screen::ScreenInfo;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

const MAX_TRANSCODE_BYTES: u64 = 256 * 1024 * 1024;

pub(super) fn bitrate(width: u32, height: u32, fps: u32) -> u64 {
    (u64::from(width) * u64::from(height) * u64::from(fps) / 4).max(1_000_000)
}

pub fn frame_rate(fps: f32, screen: &ScreenInfo) -> u32 {
    fps.floor().max(1.0).min(screen.max_fps.max(1) as f32) as u32
}

pub fn encode_h264(
    input: &Path,
    fps: f32,
    orientation: f32,
    screen: &ScreenInfo,
    control: impl Into<PreparationControl>,
) -> Result<(PathBuf, TemporaryMedia, f32), MediaError> {
    let (path, temp, fps, _) = encode_h264_with_status(
        input,
        fps,
        orientation,
        &MediaFraming::default(),
        screen,
        control,
    )?;
    Ok((path, temp, fps))
}

pub fn encode_h264_with_status(
    input: &Path,
    fps: f32,
    orientation: f32,
    framing: &MediaFraming,
    screen: &ScreenInfo,
    control: impl Into<PreparationControl>,
) -> Result<
    (
        PathBuf,
        TemporaryMedia,
        f32,
        lianli_shared::ipc::MediaEncoderStatus,
    ),
    MediaError,
> {
    let control = control.into();
    control.check()?;
    let mut temp = TemporaryMedia::new(MAX_TRANSCODE_BYTES)?;
    let output = temp.path().join("stream.h264");

    let is_image = matches!(
        input
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png" | "bmp" | "webp" | "tiff")
    );

    let (rw, rh) = render_dimensions(screen, orientation);
    let mut vf_parts = vec![super::ffmpeg::framing_filter(framing, rw, rh)];
    let rot = (orientation % 360.0 + 360.0) % 360.0;
    if (rot - 90.0).abs() < 1.0 {
        vf_parts.push("transpose=1".into());
    } else if (rot - 180.0).abs() < 1.0 {
        vf_parts.push("transpose=1,transpose=1".into());
    } else if (rot - 270.0).abs() < 1.0 {
        vf_parts.push("transpose=2".into());
    }
    let out_fps = frame_rate(fps, screen);
    vf_parts.push(format!("fps={out_fps}"));
    let vf = vf_parts.join(",");

    let (out_w, out_h) = target_dimensions(screen, orientation);
    let bitrate = bitrate(out_w, out_h, out_fps);
    let bitrate_str = format!("{bitrate}");
    let fps_str = out_fps.to_string();

    let mut last_stderr: Option<String> = None;
    for kind in encoder_chain(control.hardware_video) {
        let mut command =
            encode_command(input, &vf, &fps_str, &bitrate_str, *kind, &output, is_image);
        let output_limit = limit_output_file(&mut command, MAX_TRANSCODE_BYTES)?;
        let result = control.output(command, ENCODE_TIMEOUT).and_then(|result| {
            check_output_limit(&output, result.status, output_limit)?;
            if result.status.success() {
                Ok(())
            } else {
                Err(MediaError::Ffmpeg(
                    String::from_utf8_lossy(&result.stderr).trim().to_string(),
                ))
            }
        });
        match result {
            Ok(()) => {
                temp.retain_bytes(std::fs::metadata(&output)?.len())?;
                info!(
                    "LCD H.264 transcode: {out_w}x{out_h}@{out_fps}fps via {}",
                    kind.name()
                );
                return Ok((
                    output,
                    temp,
                    out_fps as f32,
                    kind.status(control.hardware_video),
                ));
            }
            // Only encoder-specific failures should retry the remaining backends.
            Err(MediaError::Ffmpeg(stderr)) => {
                debug!("LCD H.264 encoder {} unavailable: {stderr}", kind.name());
                last_stderr = Some(stderr);
            }
            Err(err) => return Err(err),
        }
    }

    Err(MediaError::Ffmpeg(format!(
        "All H.264 encoders failed. Last error: {}",
        last_stderr.unwrap_or_default()
    )))
}

pub(crate) fn limit_output_file(command: &mut Command, bytes: u64) -> std::io::Result<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // getrlimit writes to the initialized limit structure.
    if unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    limit.rlim_max = limit.rlim_max.min(bytes as libc::rlim_t);
    limit.rlim_cur = limit.rlim_cur.min(limit.rlim_max);
    // Only the async-signal-safe setrlimit call runs between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(limit.rlim_cur)
}

fn check_output_limit(
    path: &Path,
    status: std::process::ExitStatus,
    limit: u64,
) -> Result<(), MediaError> {
    let bytes = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !status.success() => 0,
        Err(error) => return Err(error.into()),
    };
    if bytes >= limit || status.signal() == Some(libc::SIGXFSZ) {
        return Err(MediaError::InvalidConfig(
            "H.264 video exceeds the temporary-file limit. Use a shorter clip.".into(),
        ));
    }
    if status.success() && bytes == 0 {
        return Err(MediaError::EmptyVideo);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EncoderKind {
    Nvenc,
    Amf,
    Vaapi,
    Vulkan,
    Qsv,
    Libx264,
}

impl EncoderKind {
    pub(super) fn status(self, hardware_video: bool) -> lianli_shared::ipc::MediaEncoderStatus {
        lianli_shared::ipc::MediaEncoderStatus {
            name: self.name().into(),
            software_fallback: hardware_video && self == Self::Libx264,
        }
    }
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

pub(super) fn encoder_chain(hardware_video: bool) -> &'static [EncoderKind] {
    if hardware_video {
        &[
            EncoderKind::Nvenc,
            EncoderKind::Amf,
            EncoderKind::Vaapi,
            EncoderKind::Vulkan,
            EncoderKind::Qsv,
            EncoderKind::Libx264,
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
    let mut args: Vec<String> = vec![
        "-r".into(),
        fps_str.into(),
        "-c:v".into(),
        kind.name().into(),
    ];

    match kind {
        EncoderKind::Libx264 => {
            args.extend(["-preset".into(), "ultrafast".into()]);
            args.extend(["-tune".into(), "zerolatency".into()]);
            args.extend(["-threads".into(), "1".into()]);
            args.extend(["-x264-params".into(), "slices=1".into()]);
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

fn encode_command(
    input: &Path,
    vf: &str,
    fps_str: &str,
    bitrate_str: &str,
    kind: EncoderKind,
    output: &Path,
    loop_image: bool,
) -> Command {
    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];
    args.extend(hwaccel_input_args(kind));
    if loop_image {
        args.extend(["-loop".into(), "1".into(), "-t".into(), "1".into()]);
    }
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    args.extend(["-vf".into(), finalize_vf(kind, vf)]);
    args.extend(encoder_codec_args_file(kind, fps_str, bitrate_str));
    args.extend(["-color_range".into(), "pc".into()]);
    args.extend([
        "-an".into(),
        "-f".into(),
        "h264".into(),
        output.to_string_lossy().into_owned(),
    ]);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(&args);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcode_file_limit_bounds_writes_and_rejects_truncated_success() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("stream.h264");
        for requested in [0, 1024, 8192] {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "head -c \"$2\" /dev/zero > \"$1\"; exit 0",
                "private-file-limit-fixture",
            ]);
            command.arg(&output).arg(requested.to_string());
            let limit = limit_output_file(&mut command, 4096).unwrap();
            let result = PreparationControl::from(false)
                .output(command, std::time::Duration::from_secs(5))
                .unwrap();
            assert!(result.status.success());
            assert_eq!(
                std::fs::metadata(&output).unwrap().len(),
                requested.min(4096)
            );
            let validation = check_output_limit(&output, result.status, limit);
            if requested == 0 {
                assert!(matches!(validation, Err(MediaError::EmptyVideo)));
            } else if requested < 4096 {
                validation.unwrap();
            } else {
                assert!(matches!(validation, Err(MediaError::InvalidConfig(_))));
            }
        }
    }

    #[test]
    fn disabled_policy_only_uses_software_and_enabled_policy_falls_back_last() {
        assert_eq!(encoder_chain(false), &[EncoderKind::Libx264]);
        let preferred = encoder_chain(true);
        assert_eq!(preferred.last(), Some(&EncoderKind::Libx264));
        assert!(preferred[..preferred.len() - 1]
            .iter()
            .all(|kind| *kind != EncoderKind::Libx264));
        assert!(!EncoderKind::Libx264.status(false).software_fallback);
        assert!(EncoderKind::Libx264.status(true).software_fallback);
        assert!(!EncoderKind::Nvenc.status(true).software_fallback);
    }

    #[test]
    fn framed_transcode_produces_screen_sized_stream() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("wide.png");
        image::RgbaImage::from_pixel(64, 16, image::Rgba([100, 80, 60, 255]))
            .save(&input)
            .unwrap();
        for fit in [
            lianli_shared::template::ImageFit::Cover,
            lianli_shared::template::ImageFit::Contain,
        ] {
            let framing = MediaFraming {
                fit,
                zoom: 2.0,
                offset_x: -0.5,
                offset_y: 0.5,
            };
            let (output, _temp, _, _) =
                encode_h264_with_status(&input, 10.0, 0.0, &framing, &ScreenInfo::TLLCD, false)
                    .unwrap();
            let probe = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=width,height",
                    "-of",
                    "csv=p=0",
                ])
                .arg(&output)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&probe.stdout).trim(),
                format!("{},{}", ScreenInfo::TLLCD.width, ScreenInfo::TLLCD.height)
            );
        }
    }

    #[test]
    fn software_transcode_retains_the_encoder_that_produced_the_file() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("frame.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([100, 80, 60, 255]))
            .save(&input)
            .unwrap();
        let (output, _temp, fps, encoder) = encode_h264_with_status(
            &input,
            10.9,
            0.0,
            &MediaFraming::default(),
            &ScreenInfo::TLLCD,
            false,
        )
        .unwrap();
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
        assert_eq!(fps, 10.0);
        let mut probe = Command::new("ffprobe");
        probe
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_frames",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&output);
        let inspected = PreparationControl::new(false)
            .output(probe, std::time::Duration::from_secs(5))
            .unwrap();
        assert!(inspected.status.success());
        assert_eq!(String::from_utf8(inspected.stdout).unwrap().trim(), "10");
        assert_eq!(encoder.name, "libx264");
        assert!(!encoder.software_fallback);
    }

    #[test]
    fn frame_rate_respects_fractional_limits_and_device_bounds() {
        let screen = ScreenInfo::TLLCD;
        assert_eq!(frame_rate(23.976, &screen), 23);
        assert_eq!(frame_rate(10.9, &screen), 10);
        assert_eq!(frame_rate(0.5, &screen), 1);
        assert_eq!(frame_rate(120.0, &screen), screen.max_fps);
        for fps in [1.0, 1.9, 10.9, 23.976, 29.97, 30.0, 120.0] {
            assert!(frame_rate(fps, &screen) as f32 <= fps);
        }
    }
}
