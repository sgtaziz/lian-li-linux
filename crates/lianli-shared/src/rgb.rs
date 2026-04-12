//! RGB/LED effect types shared between daemon, devices, and GUI.

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
        }
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

/// A single zone's colors within a named RGB preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbPresetZone {
    pub zone: u8,
    pub colors: Vec<[u8; 3]>,
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
