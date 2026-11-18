use crate::MAX_FRAME_BYTES;
use anyhow::{bail, ensure, Context, Result};

pub use lianli_shared::display::DisplayMode as Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// Changes since the preceding completed capture. Unknown requires a full refresh.
pub enum FrameDamage {
    #[default]
    Unknown,
    Full,
    Unchanged,
    /// Half-open bounds [left, top, right, bottom] in visible frame coordinates.
    Rectangle([u32; 4]),
}

impl FrameDamage {
    pub fn rectangle(mode: Mode, bounds: [u32; 4]) -> Result<Self> {
        let [x1, y1, x2, y2] = bounds;
        ensure!(
            x1 <= x2 && y1 <= y2 && x2 <= mode.width && y2 <= mode.height,
            "Capture damage is outside the frame"
        );
        Ok(if x1 == x2 || y1 == y2 {
            Self::Unchanged
        } else {
            Self::Rectangle(bounds)
        })
    }

    pub fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::Unchanged, value) | (value, Self::Unchanged) => value,
            (Self::Rectangle(a), Self::Rectangle(b)) => Self::Rectangle([
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureClock {
    Monotonic,
    Compositor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTimestamp {
    pub clock: CaptureClock,
    pub since_epoch: std::time::Duration,
}

impl CaptureTimestamp {
    pub fn from_parts(seconds: u64, nanos: u32, clock: CaptureClock) -> Result<Self> {
        ensure!(
            nanos < 1_000_000_000,
            "Invalid capture timestamp nanoseconds"
        );
        Ok(Self {
            clock,
            since_epoch: std::time::Duration::new(seconds, nanos),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Xbgr8888,
    Abgr8888,
}

impl PixelFormat {
    pub fn from_fourcc(value: u32) -> Result<Self> {
        match value.to_le_bytes() {
            [b'X', b'R', b'2', b'4'] => Ok(Self::Xrgb8888),
            [b'A', b'R', b'2', b'4'] => Ok(Self::Argb8888),
            [b'X', b'B', b'2', b'4'] => Ok(Self::Xbgr8888),
            [b'A', b'B', b'2', b'4'] => Ok(Self::Abgr8888),
            _ => bail!("Unsupported display pixel format {value:#x}"),
        }
    }

    pub fn rgb_byte_order(self) -> bool {
        matches!(self, Self::Xbgr8888 | Self::Abgr8888)
    }
}

pub struct Frame<'a> {
    pub mode: Mode,
    pub format: PixelFormat,
    pub stride: usize,
    pub pixels: &'a [u8],
    pub timestamp: Option<CaptureTimestamp>,
    pub damage: FrameDamage,
}

impl<'a> Frame<'a> {
    pub fn new(mode: Mode, format: PixelFormat, stride: usize, pixels: &'a [u8]) -> Result<Self> {
        mode.validate()?;
        let row = mode.width as usize * 4;
        ensure!(stride >= row, "Display stride is smaller than a pixel row");
        let required = stride
            .checked_mul(mode.height as usize - 1)
            .and_then(|bytes| bytes.checked_add(row))
            .context("Display frame layout overflow")?;
        ensure!(required <= MAX_FRAME_BYTES, "Display frame exceeds 64 MiB");
        ensure!(pixels.len() >= required, "Truncated display frame");
        Ok(Self {
            mode,
            format,
            stride,
            pixels: &pixels[..required],
            timestamp: None,
            damage: FrameDamage::Unknown,
        })
    }

    pub fn with_timestamp(mut self, timestamp: Option<CaptureTimestamp>) -> Self {
        self.timestamp = timestamp;
        self
    }

    pub fn with_damage(mut self, damage: FrameDamage) -> Result<Self> {
        self.damage = match damage {
            FrameDamage::Rectangle(bounds) => FrameDamage::rectangle(self.mode, bounds)?,
            other => other,
        };
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputRequest;

    #[test]
    fn damage_validates_bounds_and_accumulates_disjoint_updates() {
        let mode = Mode {
            width: 1920,
            height: 480,
            refresh_hz: 30,
        };
        let first = FrameDamage::rectangle(mode, [0, 10, 20, 30]).unwrap();
        let second = FrameDamage::rectangle(mode, [1900, 450, 1920, 480]).unwrap();
        assert_eq!(
            first.union(second),
            FrameDamage::Rectangle([0, 10, 1920, 480])
        );
        assert_eq!(FrameDamage::Unchanged.union(first), first);
        assert_eq!(first.union(FrameDamage::Full), FrameDamage::Full);
        assert_eq!(first.union(FrameDamage::Unknown), FrameDamage::Unknown);
        assert_eq!(
            FrameDamage::rectangle(mode, [1920, 480, 1920, 480]).unwrap(),
            FrameDamage::Unchanged
        );
        for bounds in [
            [20, 10, 0, 30],
            [0, 30, 20, 10],
            [0, 0, 1921, 480],
            [0, 0, 1920, 481],
        ] {
            assert!(FrameDamage::rectangle(mode, bounds).is_err());
        }
        let mode = Mode {
            width: 1,
            height: 1,
            refresh_hz: 30,
        };
        let frame = Frame::new(mode, PixelFormat::Xrgb8888, 4, &[0; 4]).unwrap();
        assert_eq!(frame.damage, FrameDamage::Unknown);
        assert!(frame
            .with_damage(FrameDamage::Rectangle([0, 0, 2, 1]))
            .is_err());
    }

    #[test]
    fn capture_timestamps_preserve_clock_and_full_seconds_without_overflow() {
        let timestamp =
            CaptureTimestamp::from_parts(u64::MAX, 999_999_999, CaptureClock::Compositor).unwrap();
        assert_eq!(timestamp.since_epoch.as_secs(), u64::MAX);
        assert_eq!(timestamp.since_epoch.subsec_nanos(), 999_999_999);
        assert!(CaptureTimestamp::from_parts(0, 1_000_000_000, CaptureClock::Compositor).is_err());
        assert_ne!(
            timestamp,
            CaptureTimestamp {
                clock: CaptureClock::Monotonic,
                ..timestamp
            }
        );
        let mode = Mode {
            width: 1,
            height: 1,
            refresh_hz: 30,
        };
        let frame = Frame::new(mode, PixelFormat::Xrgb8888, 4, &[0; 4]).unwrap();
        assert!(frame.timestamp.is_none());
        assert_eq!(
            frame.with_timestamp(Some(timestamp)).timestamp,
            Some(timestamp)
        );
    }

    #[test]
    fn validates_padded_frames_and_rejects_invalid_layouts() {
        let mode = Mode {
            width: 2,
            height: 3,
            refresh_hz: 60,
        };
        let bytes = [0; 40];
        let frame = Frame::new(mode, PixelFormat::Xrgb8888, 16, &bytes).unwrap();
        assert_eq!(frame.pixels.len(), 40);
        assert!(Frame::new(mode, PixelFormat::Xrgb8888, 16, &bytes[..39]).is_err());
        assert!(Frame::new(mode, PixelFormat::Xrgb8888, 7, &bytes).is_err());
        assert!(Frame::new(mode, PixelFormat::Xrgb8888, usize::MAX, &bytes).is_err());
        assert!(Mode {
            width: u32::MAX,
            height: u32::MAX,
            refresh_hz: 60
        }
        .validate()
        .is_err());
    }

    #[test]
    fn preserves_supported_low_refresh_and_rejects_unadvertised_modes() {
        let mode = Mode {
            width: 480,
            height: 480,
            refresh_hz: 15,
        };
        let request = OutputRequest {
            edid: vec![],
            preferred: mode,
            modes: vec![mode],
            max_width: 480,
            max_height: 480,
            max_frame_bytes: None,
        };
        assert!(request.validate_mode(mode).is_ok());
        assert!(request
            .validate_mode(Mode {
                refresh_hz: 30,
                ..mode
            })
            .is_err());
        assert!(request.validate_mode(Mode { width: 481, ..mode }).is_err());
        assert!(Mode {
            refresh_hz: 0,
            ..mode
        }
        .validate()
        .is_err());
        assert!(Mode {
            refresh_hz: 128,
            ..mode
        }
        .validate()
        .is_err());
    }

    #[test]
    fn fourcc_preserves_channel_order_and_rejects_other_formats() {
        for code in [b"XR24", b"AR24"] {
            assert!(!PixelFormat::from_fourcc(u32::from_le_bytes(*code))
                .unwrap()
                .rgb_byte_order());
        }
        for code in [b"XB24", b"AB24"] {
            assert!(PixelFormat::from_fourcc(u32::from_le_bytes(*code))
                .unwrap()
                .rgb_byte_order());
        }
        assert!(PixelFormat::from_fourcc(u32::from_le_bytes(*b"NV12")).is_err());
    }
}
