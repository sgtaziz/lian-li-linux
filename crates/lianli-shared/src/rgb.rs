//! RGB/LED effect types shared between daemon, devices, and GUI.

use crate::device_id::DeviceFamily;
use serde::{Deserialize, Serialize};

/// Supported RGB effect modes.
///
/// These map to hardware-native modes for wired devices (TL Fan, ENE 6K77, Galahad2).
/// For wireless devices, effects are host-rendered and streamed as RGB frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RgbMode {
    Off,
    Direct,          // Per-LED control (used by OpenRGB UpdateLEDs)
    Static,          // Solid color
    Rainbow,         // 1
    RainbowMorph,    // 2
    Breathing,       // 4
    Runway,          // 5
    Meteor,          // 6
    ColorCycle,      // 7
    Staggered,       // 8
    Tide,            // 9
    Mixing,          // 10
    Voice,           // 11
    Door,            // 12
    Render,          // 13
    Ripple,          // 14
    Reflect,         // 15
    TailChasing,     // 16
    Paint,           // 17
    PingPong,        // 18
    Stack,           // 19
    CoverCycle,      // 20
    Wave,            // 21
    Racing,          // 22
    Lottery,         // 23
    Intertwine,      // 24
    MeteorShower,    // 25
    Collide,         // 26
    ElectricCurrent, // 27
    Kaleidoscope,    // 28
    // Pump-specific modes (Galahad2 Trinity)
    BigBang,
    Vortex,
    Pump,
    ColorsMorph,
    TaiChi,
    CrossingOver,
    ColorfulStarryNight,
    StaticStarryNight,
    Bounce,
    // HydroShift LCD / Galahad2 LCD / Galahad2 Vision
    TickerTape,
    Fluctuation,
    Transmit,
    Burst,
}

impl RgbMode {
    /// Map to TL Fan / Galahad2 mode byte (1-28+). Returns None for non-mappable modes.
    pub fn to_tl_mode_byte(self) -> Option<u8> {
        match self {
            Self::Rainbow => Some(1),
            Self::RainbowMorph => Some(2),
            Self::Static => Some(3),
            Self::Breathing => Some(4),
            Self::Runway => Some(5),
            Self::Meteor => Some(6),
            Self::ColorCycle => Some(7),
            Self::Staggered => Some(8),
            Self::Tide => Some(9),
            Self::Mixing => Some(10),
            Self::Voice => Some(11),
            Self::Door => Some(12),
            Self::Render => Some(13),
            Self::Ripple => Some(14),
            Self::Reflect => Some(15),
            Self::TailChasing => Some(16),
            Self::Paint => Some(17),
            Self::PingPong => Some(18),
            Self::Stack => Some(19),
            Self::CoverCycle => Some(20),
            Self::Wave => Some(21),
            Self::Racing => Some(22),
            Self::Lottery => Some(23),
            Self::Intertwine => Some(24),
            Self::MeteorShower => Some(25),
            Self::Collide => Some(26),
            Self::ElectricCurrent => Some(27),
            Self::Kaleidoscope => Some(28),
            _ => None,
        }
    }

    pub fn to_galahad2_mode_byte(self) -> Option<u8> {
        match self {
            Self::Rainbow => Some(1),
            Self::RainbowMorph => Some(2),
            Self::Static => Some(3),
            Self::Breathing => Some(4),
            Self::Runway => Some(5),
            Self::Meteor => Some(6),
            Self::Vortex => Some(7),
            Self::CrossingOver => Some(8),
            Self::TaiChi => Some(9),
            Self::ColorfulStarryNight => Some(10),
            Self::StaticStarryNight => Some(11),
            Self::Voice => Some(12),
            Self::BigBang => Some(13),
            Self::Pump => Some(14),
            Self::ColorsMorph => Some(15),
            Self::Bounce => Some(16),
            _ => None,
        }
    }

    pub fn to_hydroshift_lcd_mode_byte(self) -> Option<u8> {
        match self {
            Self::Rainbow => Some(1),
            Self::RainbowMorph => Some(2),
            Self::Static => Some(3),
            Self::Breathing => Some(4),
            Self::Runway => Some(5),
            Self::Meteor => Some(6),
            Self::TickerTape => Some(7),
            Self::Fluctuation => Some(8),
            Self::Transmit => Some(9),
            Self::ColorfulStarryNight => Some(10),
            Self::StaticStarryNight => Some(11),
            Self::Voice => Some(12),
            Self::BigBang => Some(13),
            Self::Burst => Some(14),
            Self::ColorsMorph => Some(15),
            Self::Bounce => Some(16),
            _ => None,
        }
    }

    pub fn is_valid_galahad2_pump_scope(self, scope: RgbScope) -> bool {
        let Some(byte) = self.to_galahad2_mode_byte() else {
            return false;
        };
        match scope {
            RgbScope::Inner => matches!(byte, 1..=6 | 9..=12 | 14),
            RgbScope::Outer => matches!(byte, 1..=6 | 9..=12 | 14 | 16),
            _ => matches!(byte, 1..=15),
        }
    }

    /// Map from TL Fan mode byte to RgbMode.
    pub fn from_tl_mode_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Rainbow),
            2 => Some(Self::RainbowMorph),
            3 => Some(Self::Static),
            4 => Some(Self::Breathing),
            5 => Some(Self::Runway),
            6 => Some(Self::Meteor),
            7 => Some(Self::ColorCycle),
            8 => Some(Self::Staggered),
            9 => Some(Self::Tide),
            10 => Some(Self::Mixing),
            11 => Some(Self::Voice),
            12 => Some(Self::Door),
            13 => Some(Self::Render),
            14 => Some(Self::Ripple),
            15 => Some(Self::Reflect),
            16 => Some(Self::TailChasing),
            17 => Some(Self::Paint),
            18 => Some(Self::PingPong),
            19 => Some(Self::Stack),
            20 => Some(Self::CoverCycle),
            21 => Some(Self::Wave),
            22 => Some(Self::Racing),
            23 => Some(Self::Lottery),
            24 => Some(Self::Intertwine),
            25 => Some(Self::MeteorShower),
            26 => Some(Self::Collide),
            27 => Some(Self::ElectricCurrent),
            28 => Some(Self::Kaleidoscope),
            _ => None,
        }
    }

    /// Inverse of `to_galahad2_mode_byte`.
    pub fn from_galahad2_mode_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Rainbow),
            2 => Some(Self::RainbowMorph),
            3 => Some(Self::Static),
            4 => Some(Self::Breathing),
            5 => Some(Self::Runway),
            6 => Some(Self::Meteor),
            7 => Some(Self::Vortex),
            8 => Some(Self::CrossingOver),
            9 => Some(Self::TaiChi),
            10 => Some(Self::ColorfulStarryNight),
            11 => Some(Self::StaticStarryNight),
            12 => Some(Self::Voice),
            13 => Some(Self::BigBang),
            14 => Some(Self::Pump),
            15 => Some(Self::ColorsMorph),
            16 => Some(Self::Bounce),
            _ => None,
        }
    }

    /// Inverse of `to_hydroshift_lcd_mode_byte`.
    pub fn from_hydroshift_lcd_mode_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Rainbow),
            2 => Some(Self::RainbowMorph),
            3 => Some(Self::Static),
            4 => Some(Self::Breathing),
            5 => Some(Self::Runway),
            6 => Some(Self::Meteor),
            7 => Some(Self::TickerTape),
            8 => Some(Self::Fluctuation),
            9 => Some(Self::Transmit),
            10 => Some(Self::ColorfulStarryNight),
            11 => Some(Self::StaticStarryNight),
            12 => Some(Self::Voice),
            13 => Some(Self::BigBang),
            14 => Some(Self::Burst),
            15 => Some(Self::ColorsMorph),
            16 => Some(Self::Bounce),
            _ => None,
        }
    }

    /// Family-aware mode-byte lookup.
    ///
    /// Single entry point for driver code: pass the device family, get the
    /// hardware byte (or `None` if the mode isn't supported by that family).
    pub fn mode_byte_for(self, family: DeviceFamily) -> Option<u8> {
        match family {
            DeviceFamily::TlFan
            | DeviceFamily::Ene6k77
            | DeviceFamily::SlInf
            | DeviceFamily::Clv1 => self.to_tl_mode_byte(),
            DeviceFamily::Galahad2Trinity => self.to_galahad2_mode_byte(),
            DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd | DeviceFamily::WirelessAio => {
                self.to_hydroshift_lcd_mode_byte()
            }
            _ => None,
        }
    }

    /// Family-aware inverse of `mode_byte_for`.
    pub fn from_mode_byte_for(family: DeviceFamily, byte: u8) -> Option<Self> {
        match family {
            DeviceFamily::TlFan
            | DeviceFamily::Ene6k77
            | DeviceFamily::SlInf
            | DeviceFamily::Clv1 => Self::from_tl_mode_byte(byte),
            DeviceFamily::Galahad2Trinity => Self::from_galahad2_mode_byte(byte),
            DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd | DeviceFamily::WirelessAio => {
                Self::from_hydroshift_lcd_mode_byte(byte)
            }
            _ => None,
        }
    }

    /// Display name for GUI.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Direct => "Direct",
            Self::Static => "Static",
            Self::Rainbow => "Rainbow",
            Self::RainbowMorph => "Rainbow Morph",
            Self::Breathing => "Breathing",
            Self::Runway => "Runway",
            Self::Meteor => "Meteor",
            Self::ColorCycle => "Color Cycle",
            Self::Staggered => "Staggered",
            Self::Tide => "Tide",
            Self::Mixing => "Mixing",
            Self::Voice => "Voice",
            Self::Door => "Door",
            Self::Render => "Render",
            Self::Ripple => "Ripple",
            Self::Reflect => "Reflect",
            Self::TailChasing => "Tail Chasing",
            Self::Paint => "Paint",
            Self::PingPong => "Ping Pong",
            Self::Stack => "Stack",
            Self::CoverCycle => "Cover Cycle",
            Self::Wave => "Wave",
            Self::Racing => "Racing",
            Self::Lottery => "Lottery",
            Self::Intertwine => "Intertwine",
            Self::MeteorShower => "Meteor Shower",
            Self::Collide => "Collide",
            Self::ElectricCurrent => "Electric Current",
            Self::Kaleidoscope => "Kaleidoscope",
            Self::BigBang => "Big Bang",
            Self::Vortex => "Vortex",
            Self::Pump => "Pump",
            Self::ColorsMorph => "Colors Morph",
            Self::TaiChi => "Tai Chi",
            Self::CrossingOver => "Crossing Over",
            Self::ColorfulStarryNight => "Colorful Starry Night",
            Self::StaticStarryNight => "Static Starry Night",
            Self::Bounce => "Bounce",
            Self::TickerTape => "Ticker Tape",
            Self::Fluctuation => "Fluctuation",
            Self::Transmit => "Transmit",
            Self::Burst => "Burst",
        }
    }

    /// Inverse of `display_name` — parse a mode from its GUI string.
    ///
    /// Used by the OpenRGB server (which receives mode names back from clients)
    /// and by the GUI when reading a serialized preset. Case-sensitive; pass
    /// strings produced by `display_name()` for guaranteed round-trip.
    pub fn from_display_name(name: &str) -> Option<Self> {
        if name.is_empty() {
            return None;
        }
        // Off is intentionally NOT in this list: it is a sentinel used by the
        // GUI but the OpenRGB protocol never sends "Off" — it sends brightness 0.
        Some(match name {
            "Direct" => Self::Direct,
            "Static" => Self::Static,
            "Rainbow" => Self::Rainbow,
            "Rainbow Morph" => Self::RainbowMorph,
            "Breathing" => Self::Breathing,
            "Runway" => Self::Runway,
            "Meteor" => Self::Meteor,
            "Color Cycle" => Self::ColorCycle,
            "Staggered" => Self::Staggered,
            "Tide" => Self::Tide,
            "Mixing" => Self::Mixing,
            "Voice" => Self::Voice,
            "Door" => Self::Door,
            "Render" => Self::Render,
            "Ripple" => Self::Ripple,
            "Reflect" => Self::Reflect,
            "Tail Chasing" => Self::TailChasing,
            "Paint" => Self::Paint,
            "Ping Pong" => Self::PingPong,
            "Stack" => Self::Stack,
            "Cover Cycle" => Self::CoverCycle,
            "Wave" => Self::Wave,
            "Racing" => Self::Racing,
            "Lottery" => Self::Lottery,
            "Intertwine" => Self::Intertwine,
            "Meteor Shower" => Self::MeteorShower,
            "Collide" => Self::Collide,
            "Electric Current" => Self::ElectricCurrent,
            "Kaleidoscope" => Self::Kaleidoscope,
            "Big Bang" => Self::BigBang,
            "Vortex" => Self::Vortex,
            "Pump" => Self::Pump,
            "Colors Morph" => Self::ColorsMorph,
            "Tai Chi" => Self::TaiChi,
            "Crossing Over" => Self::CrossingOver,
            "Colorful Starry Night" => Self::ColorfulStarryNight,
            "Static Starry Night" => Self::StaticStarryNight,
            "Bounce" => Self::Bounce,
            "Ticker Tape" => Self::TickerTape,
            "Fluctuation" => Self::Fluctuation,
            "Transmit" => Self::Transmit,
            "Burst" => Self::Burst,
            _ => return None,
        })
    }
}

/// Effect animation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RgbDirection {
    #[default]
    Clockwise,
    CounterClockwise,
    Up,
    Down,
    Spread,
    Gather,
}

impl RgbDirection {
    /// Map to TL Fan / Galahad2 direction byte.
    pub fn to_tl_byte(self) -> u8 {
        match self {
            Self::Clockwise => 0,
            Self::CounterClockwise => 1,
            Self::Up => 2,
            Self::Down => 3,
            Self::Spread => 4,
            Self::Gather => 5,
        }
    }

    /// Map to ENE 6K77 direction byte (only Left/Right).
    pub fn to_ene_byte(self) -> u8 {
        match self {
            Self::CounterClockwise => 1, // Left
            _ => 0,                      // Right (default)
        }
    }
}

/// RGB effect scope (which LEDs are targeted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RgbScope {
    #[default]
    All,
    Top,
    Bottom,
    Inner,
    Outer,
}

/// A complete RGB effect definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbEffect {
    pub mode: RgbMode,
    /// Up to 4 RGB colors.
    #[serde(default = "default_colors")]
    pub colors: Vec<[u8; 3]>,
    /// Speed: 0-4 (slowest to fastest).
    #[serde(default = "default_speed")]
    pub speed: u8,
    /// Brightness: 0-4 (dimmest to brightest).
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Animation direction.
    #[serde(default)]
    pub direction: RgbDirection,
    /// Which LED scope to target (All, Top, Bottom, Inner, Outer).
    #[serde(default)]
    pub scope: RgbScope,
    /// Hardware "disabled" flag, turns LEDs off without changing mode/colors.
    #[serde(default)]
    pub disabled: bool,
}

fn default_colors() -> Vec<[u8; 3]> {
    vec![[0, 0, 0]]
}

fn default_speed() -> u8 {
    2
}

fn default_brightness() -> u8 {
    4
}

impl Default for RgbEffect {
    fn default() -> Self {
        Self {
            mode: RgbMode::Static,
            colors: vec![[255, 255, 255]],
            speed: default_speed(),
            brightness: default_brightness(),
            direction: RgbDirection::default(),
            scope: RgbScope::default(),
            disabled: false,
        }
    }
}

/// Per-zone RGB configuration (stored in config file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbZoneConfig {
    pub zone_index: u8,
    pub effect: RgbEffect,
    /// Fan orientation: swap left/right direction.
    #[serde(default)]
    pub swap_lr: bool,
    /// Fan orientation: swap top/bottom direction.
    #[serde(default)]
    pub swap_tb: bool,
}

/// Per-device RGB configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbDeviceConfig {
    pub device_id: String,
    /// Use motherboard ARGB header instead of software-controlled effects.
    #[serde(default)]
    pub mb_rgb_sync: bool,
    /// Name of the currently active RGB preset (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_preset: Option<String>,
    pub zones: Vec<RgbZoneConfig>,
}

/// Top-level RGB configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbAppConfig {
    /// Whether RGB control is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to run the OpenRGB SDK server.
    #[serde(default)]
    pub openrgb_server: bool,
    /// OpenRGB SDK server port.
    #[serde(default = "default_openrgb_port")]
    pub openrgb_port: u16,
    /// Per-device RGB settings.
    #[serde(default)]
    pub devices: Vec<RgbDeviceConfig>,
}

fn default_true() -> bool {
    true
}

fn default_openrgb_port() -> u16 {
    6743
}

impl Default for RgbAppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            openrgb_server: false,
            openrgb_port: default_openrgb_port(),
            devices: Vec::new(),
        }
    }
}

/// A single zone's state within a named RGB preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbPresetZone {
    pub zone: u8,
    #[serde(default)]
    pub colors: Vec<[u8; 3]>,
    #[serde(default)]
    pub effect: Option<RgbEffect>,
}

/// A named per-LED color preset that can be saved to config and applied later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbPreset {
    pub name: String,
    pub device_id: String,
    pub zones: Vec<RgbPresetZone>,
}

/// Information about an RGB zone, reported to GUI/OpenRGB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbZoneInfo {
    pub name: String,
    pub led_count: u16,
}

/// RGB capabilities reported per device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbDeviceCapabilities {
    pub device_id: String,
    pub device_name: String,
    pub supported_modes: Vec<RgbMode>,
    pub zones: Vec<RgbZoneInfo>,
    /// Whether this device supports per-LED direct color control.
    pub supports_direct: bool,
    /// Whether this device supports motherboard ARGB sync.
    pub supports_mb_rgb_sync: bool,
    /// Total number of LEDs across all zones.
    pub total_led_count: u16,
    /// Supported scopes per zone. Empty vec = only "All" (no selector shown).
    pub supported_scopes: Vec<Vec<RgbScope>>,
    /// Whether this device supports fan direction (swap LR/TB).
    #[serde(default)]
    pub supports_direction: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MODES: &[RgbMode] = &[
        RgbMode::Off,
        RgbMode::Direct,
        RgbMode::Static,
        RgbMode::Rainbow,
        RgbMode::RainbowMorph,
        RgbMode::Breathing,
        RgbMode::Runway,
        RgbMode::Meteor,
        RgbMode::ColorCycle,
        RgbMode::Staggered,
        RgbMode::Tide,
        RgbMode::Mixing,
        RgbMode::Voice,
        RgbMode::Door,
        RgbMode::Render,
        RgbMode::Ripple,
        RgbMode::Reflect,
        RgbMode::TailChasing,
        RgbMode::Paint,
        RgbMode::PingPong,
        RgbMode::Stack,
        RgbMode::CoverCycle,
        RgbMode::Wave,
        RgbMode::Racing,
        RgbMode::Lottery,
        RgbMode::Intertwine,
        RgbMode::MeteorShower,
        RgbMode::Collide,
        RgbMode::ElectricCurrent,
        RgbMode::Kaleidoscope,
        RgbMode::BigBang,
        RgbMode::Vortex,
        RgbMode::Pump,
        RgbMode::ColorsMorph,
        RgbMode::TaiChi,
        RgbMode::CrossingOver,
        RgbMode::ColorfulStarryNight,
        RgbMode::StaticStarryNight,
        RgbMode::Bounce,
        RgbMode::TickerTape,
        RgbMode::Fluctuation,
        RgbMode::Transmit,
        RgbMode::Burst,
    ];

    #[test]
    fn display_name_round_trip() {
        // Off is intentionally not in `from_display_name` (it's a sentinel);
        // every other mode must round-trip.
        for &mode in ALL_MODES {
            if mode == RgbMode::Off {
                assert_eq!(RgbMode::from_display_name(mode.display_name()), None);
                continue;
            }
            let parsed = RgbMode::from_display_name(mode.display_name())
                .unwrap_or_else(|| panic!("failed to parse display name {:?}", mode));
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn from_display_name_rejects_unknown() {
        assert_eq!(RgbMode::from_display_name(""), None);
        assert_eq!(RgbMode::from_display_name("Off"), None);
        assert_eq!(RgbMode::from_display_name("not a real mode"), None);
    }

    #[test]
    fn tl_mode_byte_round_trip() {
        for &mode in ALL_MODES {
            let Some(byte) = mode.to_tl_mode_byte() else {
                continue;
            };
            let back = RgbMode::from_tl_mode_byte(byte).expect("inverse missing");
            assert_eq!(back, mode, "tl mode byte round trip failed for {:?}", mode);
        }
    }

    #[test]
    fn galahad2_mode_byte_round_trip() {
        for &mode in ALL_MODES {
            let Some(byte) = mode.to_galahad2_mode_byte() else {
                continue;
            };
            let back = RgbMode::from_galahad2_mode_byte(byte).expect("inverse missing");
            assert_eq!(back, mode, "galahad2 byte round trip failed for {:?}", mode);
        }
    }

    #[test]
    fn hydroshift_lcd_mode_byte_round_trip() {
        for &mode in ALL_MODES {
            let Some(byte) = mode.to_hydroshift_lcd_mode_byte() else {
                continue;
            };
            let back = RgbMode::from_hydroshift_lcd_mode_byte(byte).expect("inverse missing");
            assert_eq!(
                back, mode,
                "hydroshift byte round trip failed for {:?}",
                mode
            );
        }
    }

    #[test]
    fn family_dispatcher_matches_direct_calls() {
        let families = [
            (DeviceFamily::TlFan, "tl"),
            (DeviceFamily::Ene6k77, "ene"),
            (DeviceFamily::SlInf, "slinf"),
            (DeviceFamily::Clv1, "clv1"),
            (DeviceFamily::Galahad2Trinity, "galahad2"),
            (DeviceFamily::HydroShiftLcd, "hydroshift"),
            (DeviceFamily::Galahad2Lcd, "galahad2lcd"),
            (DeviceFamily::WirelessAio, "w aio"),
        ];
        for &(family, _) in &families {
            for &mode in ALL_MODES {
                let dispatched = mode.mode_byte_for(family);
                let direct = match family {
                    DeviceFamily::TlFan
                    | DeviceFamily::Ene6k77
                    | DeviceFamily::SlInf
                    | DeviceFamily::Clv1 => mode.to_tl_mode_byte(),
                    DeviceFamily::Galahad2Trinity => mode.to_galahad2_mode_byte(),
                    DeviceFamily::HydroShiftLcd
                    | DeviceFamily::Galahad2Lcd
                    | DeviceFamily::WirelessAio => mode.to_hydroshift_lcd_mode_byte(),
                    _ => None,
                };
                assert_eq!(
                    dispatched, direct,
                    "dispatcher mismatch for {:?} / {:?}",
                    family, mode
                );

                if let Some(byte) = dispatched {
                    let back = RgbMode::from_mode_byte_for(family, byte).unwrap();
                    assert_eq!(back, mode);
                }
            }
        }
    }

    #[test]
    fn family_dispatcher_returns_none_for_unsupported_families() {
        // LCD-only / RGB-only families have no mode byte table.
        for family in [
            DeviceFamily::TlLcd,
            DeviceFamily::UniversalScreenLighting,
            DeviceFamily::StrimerPlus,
            DeviceFamily::WirelessTx,
        ] {
            assert_eq!(RgbMode::Static.mode_byte_for(family), None);
            assert_eq!(RgbMode::from_mode_byte_for(family, 1), None);
        }
    }
}
