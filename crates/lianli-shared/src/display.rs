use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SESSION_DISPLAYS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopEncoder {
    Turbojpeg,
    Libx264,
    H264Vaapi,
    H264Nvenc,
    H264Amf,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuReadbackReason {
    NoDmaBuf,
    GpuFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareEncodingReason {
    HardwareUnavailable,
    HardwareFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopEncodingStatus {
    pub encoder: DesktopEncoder,
    pub gpu_input: bool,
    pub cpu_readback_reason: Option<CpuReadbackReason>,
    pub software_reason: Option<SoftwareEncodingReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

impl DisplayMode {
    pub fn validate(self) -> Result<()> {
        ensure!(self.width > 0 && self.height > 0, "Empty display mode");
        ensure!(
            (1..=127).contains(&self.refresh_hz),
            "Unsupported display refresh rate"
        );
        let pixels = u64::from(self.width) * u64::from(self.height);
        ensure!(
            pixels <= MAX_FRAME_BYTES as u64 / 4,
            "Display frame exceeds 64 MiB"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputRequest {
    pub edid: Vec<u8>,
    pub preferred: DisplayMode,
    pub modes: Vec<DisplayMode>,
    pub max_width: u32,
    pub max_height: u32,
    /// Largest JPEG frame to send in one transfer (descriptor max_transfer minus framing).
    #[serde(default)]
    pub max_frame_bytes: Option<u32>,
}

impl OutputRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.edid.len(), 128 | 256),
            "Unsupported EDID size"
        );
        ensure!(
            !self.modes.is_empty() && self.modes.len() <= 64,
            "Invalid display mode count"
        );
        for mode in &self.modes {
            self.validate_mode(*mode)?;
        }
        if let Some(bytes) = self.max_frame_bytes {
            ensure!(
                (1..=MAX_ENCODED_BYTES as u32).contains(&bytes),
                "Invalid encoded frame limit"
            );
        }
        self.validate_mode(self.preferred)
    }

    pub fn validate_mode(&self, mode: DisplayMode) -> Result<()> {
        mode.validate()?;
        ensure!(
            mode.width <= self.max_width && mode.height <= self.max_height,
            "Display mode exceeds the physical panel's dimensions"
        );
        ensure!(
            self.modes.contains(&mode),
            "Display mode is not advertised by the physical panel"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayCodec {
    Jpeg,
    H264,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayVideoPolicy {
    pub hardware_video: bool,
    pub fps_limit: u32,
}

impl DisplayVideoPolicy {
    pub fn fps(self, source_fps: u32) -> u32 {
        source_fps.clamp(1, 127).min(self.fps_limit.clamp(1, 120))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerHello {
    pub session_id: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerCommand {
    Registered,
    Open {
        id: u64,
        output: OutputRequest,
        codec: DisplayCodec,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerClosed {
    pub id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureCommand {
    Next {
        sequence: u64,
        generation: u64,
        policy: DisplayVideoPolicy,
    },
    Pause,
    Close,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureReply {
    Ready {
        backend: String,
        #[serde(default)]
        fallback_reason: Option<String>,
        buffer_bytes: usize,
    },
    Frame {
        sequence: u64,
        generation: u64,
        mode: DisplayMode,
        bytes: usize,
        #[serde(default)]
        encoding: Option<DesktopEncodingStatus>,
    },
    Idle {
        sequence: u64,
    },
    Paused,
    Power {
        powered: bool,
    },
    Failed {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_excessive_frames_and_unadvertised_worker_modes() {
        assert!(DisplayMode {
            width: u32::MAX,
            height: u32::MAX,
            refresh_hz: 60
        }
        .validate()
        .is_err());
        let mode = DisplayMode {
            width: 480,
            height: 480,
            refresh_hz: 24,
        };
        let mut output = OutputRequest {
            edid: vec![0; 128],
            preferred: mode,
            modes: vec![mode],
            max_width: 480,
            max_height: 480,
            max_frame_bytes: None,
        };
        output.validate().unwrap();
        output.preferred.refresh_hz = 60;
        assert!(output.validate().is_err());
        output.preferred = mode;
        output.max_frame_bytes = Some(131_030);
        output.validate().unwrap();
        output.max_frame_bytes = Some(0);
        assert!(output.validate().is_err());
        output.max_frame_bytes = Some(MAX_ENCODED_BYTES as u32 + 1);
        assert!(output.validate().is_err());
        output.max_frame_bytes = None;
        output.edid.resize(1024, 0);
        assert!(output.validate().is_err());
    }
}
