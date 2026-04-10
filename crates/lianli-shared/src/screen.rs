use crate::device_id::DeviceFamily;

/// Screen resolution and streaming parameters for LCD devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub jpeg_quality: u8,
    pub max_payload: usize,
    pub device_rotation: u16,
    pub h264: bool,
}

impl ScreenInfo {
    /// SLV3 / TLV2 wireless LCD fans (400x400, USB bulk, DES-encrypted header).
    pub const WIRELESS_LCD: Self = Self {
        width: 400,
        height: 400,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 102_400 - 512,
        device_rotation: 0,
        h264: false,
    };

    pub const TLLCD: Self = Self {
        width: 400,
        height: 400,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 65_535,
        device_rotation: 0,
        h264: false,
    };

    pub const AIO_LCD_480: Self = Self {
        width: 480,
        height: 480,
        max_fps: 24,
        jpeg_quality: 85,
        max_payload: 153_600,
        device_rotation: 0,
        h264: false,
    };

    pub const HYDROSHIFT2: Self = Self {
        width: 480,
        height: 480,
        max_fps: 24,
        jpeg_quality: 85,
        max_payload: 153_600,
        device_rotation: 0,
        h264: true,
    };

    pub const LANCOOL_207: Self = Self {
        width: 1472,
        height: 720,
        max_fps: 30,
        jpeg_quality: 95,
        max_payload: 512_000,
        device_rotation: 90,
        h264: true,
    };

    pub const UNIVERSAL_SCREEN: Self = Self {
        width: 480,
        height: 1920,
        max_fps: 30,
        jpeg_quality: 95,
        max_payload: 512_000,
        device_rotation: 0,
        h264: true,
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
        DeviceFamily::Lancool207 => Some(ScreenInfo::LANCOOL_207),
        DeviceFamily::UniversalScreen => Some(ScreenInfo::UNIVERSAL_SCREEN),
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
            label: "Lancool 207 (1472×720)",
            width: 1472,
            height: 720,
        },
        ScreenPreset {
            label: "Universal Screen 8.8\" (480×1920)",
            width: 480,
            height: 1920,
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
