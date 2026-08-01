use crate::device_id::DeviceFamily;

/// Screen resolution and streaming parameters for LCD devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub jpeg_quality: u8,
    pub max_payload: usize,
    pub h264: bool,
    pub needs_keepalive: bool,
    /// Push frames as PNG (opcode 0x66 overlay layer) instead of JPEG (0x65
    /// background layer). Used by the HS2 OLED Curve, whose firmware renders
    /// the PNG layer on the OLED panel.
    pub png: bool,
}

impl ScreenInfo {
    /// SLV3 / TLV2 wireless LCD fans (400x400, USB bulk, DES-encrypted header).
    pub const WIRELESS_LCD: Self = Self {
        width: 400,
        height: 400,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 102_400 - 512,
        h264: false,
        needs_keepalive: false,
        png: false,
    };

    pub const TLLCD: Self = Self {
        width: 400,
        height: 400,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 65_535,
        h264: false,
        needs_keepalive: true,
        png: false,
    };

    pub const AIO_LCD_480: Self = Self {
        width: 480,
        height: 480,
        max_fps: 24,
        jpeg_quality: 85,
        max_payload: 153_600,
        h264: true,
        needs_keepalive: false,
        png: false,
    };

    pub const HYDROSHIFT2: Self = Self {
        width: 480,
        height: 480,
        max_fps: 24,
        jpeg_quality: 85,
        max_payload: 153_600,
        h264: true,
        needs_keepalive: false,
        png: false,
    };

    /// HydroShift II OLED Curve (0x1CBE:0xA068) — 1080×2288 OLED panel.
    pub const HYDROSHIFT2_OLED_CURVE: Self = Self {
        width: 1080,
        height: 2288,
        max_fps: 30,
        jpeg_quality: 95,
        max_payload: 1_048_576,
        h264: true,
        needs_keepalive: false,
        png: true,
    };

    pub const LANCOOL_207: Self = Self {
        width: 720,
        height: 1472,
        max_fps: 24,
        jpeg_quality: 95,
        max_payload: 512_000,
        h264: false,
        needs_keepalive: false,
        png: false,
    };

    pub const UNIVERSAL_SCREEN: Self = Self {
        width: 480,
        height: 1920,
        max_fps: 24,
        jpeg_quality: 95,
        max_payload: 512_000,
        h264: true,
        needs_keepalive: false,
        png: false,
    };

    pub const VISION_9P2: Self = Self {
        width: 464,
        height: 1920,
        max_fps: 24,
        jpeg_quality: 95,
        max_payload: 512_000,
        h264: false,
        needs_keepalive: false,
        png: false,
    };

    pub const FLEX_LCD: Self = Self {
        width: 400,
        height: 400,
        max_fps: 24,
        jpeg_quality: 95,
        max_payload: 512_000,
        h264: true,
        needs_keepalive: false,
        png: false,
    };
}

/// Get the screen info for a given device family.
/// Returns `None` for devices that don't have LCDs.
pub fn screen_info_for(family: DeviceFamily) -> Option<ScreenInfo> {
    match family {
        DeviceFamily::Slv3Lcd | DeviceFamily::Tlv2Lcd => Some(ScreenInfo::WIRELESS_LCD),
        DeviceFamily::TlLcd => Some(ScreenInfo::TLLCD),
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => Some(ScreenInfo::AIO_LCD_480),
        DeviceFamily::HydroShift2Lcd => Some(ScreenInfo::HYDROSHIFT2),
        DeviceFamily::HydroShift2OledCurveLcd => Some(ScreenInfo::HYDROSHIFT2_OLED_CURVE),
        DeviceFamily::Lancool207 => Some(ScreenInfo::LANCOOL_207),
        DeviceFamily::UniversalScreen => Some(ScreenInfo::UNIVERSAL_SCREEN),
        DeviceFamily::Vision9p2 => Some(ScreenInfo::VISION_9P2),
        DeviceFamily::TlFlexLcd | DeviceFamily::SlInfFlexLcd => Some(ScreenInfo::FLEX_LCD),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScreenPreset {
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
}

pub fn screen_presets() -> &'static [ScreenPreset] {
    &[
        ScreenPreset {
            label: "Wireless LCD / TL LCD (400×400)",
            width: 400,
            height: 400,
        },
        ScreenPreset {
            label: "AIO LCD / HydroShift 2 (480×480)",
            width: 480,
            height: 480,
        },
        ScreenPreset {
            label: "HydroShift II OLED Curve (1080×2288)",
            width: 1080,
            height: 2288,
        },
        ScreenPreset {
            label: "Lancool 207 (1472×720)",
            width: 1472,
            height: 720,
        },
        ScreenPreset {
            label: "Universal Screen 8.8\" (480×1920)",
            width: 480,
            height: 1920,
        },
        ScreenPreset {
            label: "Vision 9.2\" (464×1920)",
            width: 464,
            height: 1920,
        },
        ScreenPreset {
            label: "Flex LCD (480×480)",
            width: 480,
            height: 480,
        },
    ]
}

pub fn screen_preset_label(width: u32, height: u32) -> String {
    for preset in screen_presets() {
        if preset.width == width && preset.height == height {
            return preset.label.to_string();
        }
    }
    format!("Custom {width}×{height}")
}
