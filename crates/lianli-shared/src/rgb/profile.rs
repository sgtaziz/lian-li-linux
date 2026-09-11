use serde::{Deserialize, Serialize};

pub const MAX_RGB_ANIMATION_FRAMES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RgbRenderFamily {
    Tl,
    Sl,
    SlInf,
    SlInfV3,
    SlV4,
    Cl,
    P28,
    Strimer,
    HydroShiftII,
    HydroShiftIIOled,
    UniversalScreen,
    Lancool217,
    LancoolV150,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbRenderProfile {
    pub family: RgbRenderFamily,
    pub fan_count: u8,
    pub led_count: u16,
    #[serde(default)]
    pub right_attach: bool,
}

impl RgbRenderProfile {
    pub fn sync_frame_led_count(self) -> u16 {
        match self.family {
            RgbRenderFamily::UniversalScreen => 60,
            RgbRenderFamily::HydroShiftIIOled => 35,
            _ => self.led_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RgbPlaybackTiming {
    /// Primary interval in hundredths of the vendor's 0.625 ms clock tick.
    pub interval_hundredths: u32,
    pub secondary_interval_ticks: u16,
    pub secondary_frame_count: u16,
    pub outer_longest: bool,
}

impl RgbPlaybackTiming {
    pub fn from_millis(interval_ms: u16) -> Self {
        Self {
            interval_hundredths: u32::from(interval_ms) * 160,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_without_attachment_deserialize_as_left_attached() {
        let profile: RgbRenderProfile =
            serde_json::from_str(r#"{"family":"SlInf","fan_count":3,"led_count":132}"#).unwrap();
        assert!(!profile.right_attach);
        assert_eq!(profile.family, RgbRenderFamily::SlInf);
        let right = RgbRenderProfile {
            right_attach: true,
            family: RgbRenderFamily::SlInfV3,
            ..profile
        };
        assert_eq!(
            serde_json::from_str::<RgbRenderProfile>(&serde_json::to_string(&right).unwrap())
                .unwrap(),
            right
        );
    }

    #[test]
    fn sync_frame_counts_match_transmitted_buffers() {
        let profile = |family, led_count| RgbRenderProfile {
            family,
            fan_count: 0,
            led_count,
            right_attach: false,
        };

        assert_eq!(
            profile(RgbRenderFamily::SlV4, 104).sync_frame_led_count(),
            104
        );
        assert_eq!(profile(RgbRenderFamily::P28, 27).sync_frame_led_count(), 27);
        assert_eq!(
            profile(RgbRenderFamily::Strimer, 116).sync_frame_led_count(),
            116
        );
        assert_eq!(
            profile(RgbRenderFamily::UniversalScreen, 88).sync_frame_led_count(),
            60
        );
        assert_eq!(
            profile(RgbRenderFamily::HydroShiftIIOled, 45).sync_frame_led_count(),
            35
        );
    }
}
