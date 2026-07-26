/// Wireless fan device type, determines minimum duty and RPM curves.
///
/// Byte ranges for classifying fan type:
/// ```text
/// SLV3  (base 20): 20-26  (LED: 20-23, LCD: 24-26)
/// TLV2  (base 27): 27-35  (LCD: 27,32-35, LED: 28-31)
/// SLINF (base 36): 36-39  (LED only)
/// RL120:           40
/// CLV1:            41-42
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WirelessFanType {
    /// SLV3 120mm/140mm LED fans (no LCD) — 14% minimum duty
    Slv3Led,
    /// SLV3 120mm/140mm LCD fans — 14% minimum duty
    Slv3Lcd,
    /// TLV2 120mm/140mm LCD fans — 10% minimum duty
    Tlv2Lcd,
    /// TLV2 120mm/140mm LED fans (no LCD) — 11% minimum duty
    Tlv2Led,
    /// SL-INF wireless fans — 11% minimum duty
    SlInf,
    /// CL / RL120 fans — 10% minimum duty (special PWM filter)
    Clv1,
    /// HydroShift II LCD-C (Circle) wireless AIO (device_type 10).
    /// Pump RPM range 1600-2500, 0-4 fans, 24 LEDs on pump head.
    WaterBlock,
    /// HydroShift II LCD-S / H2S (Square) wireless AIO (device_type 11).
    /// Pump RPM range 1600-3200, 0-4 fans, 24 LEDs on pump head.
    WaterBlock2,
    /// Wireless LED strip (device_type 1-9) — RGB only, no fans
    Strimer(u8),
    /// Lancool 217 case RGB ring (device_type 65) — 96 LEDs, no fans
    Lc217,
    /// Universal Screen 8.8" LED ring (device_type 88) — 88 LEDs, no fans
    Led88,
    /// Lancool V150 case fan/RGB controller (device_type 66) — 88 LEDs, dual-zone front/rear
    V150,
    /// Unknown fan type
    Unknown,
}

impl WirelessFanType {
    /// Minimum duty percentage for this fan type.
    pub fn min_duty_percent(self) -> u8 {
        match self {
            Self::Slv3Led | Self::Slv3Lcd => 14,
            Self::Tlv2Lcd => 10,
            Self::Tlv2Led | Self::SlInf => 11,
            Self::Clv1 | Self::WaterBlock | Self::WaterBlock2 | Self::V150 => 10,
            Self::Strimer(_) | Self::Lc217 | Self::Led88 => 0,
            Self::Unknown => 10,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Slv3Led => "UNI FAN SL V3 Wireless",
            Self::Slv3Lcd => "UNI FAN SL V3 Wireless LCD",
            Self::Tlv2Lcd => "UNI FAN TL Wireless LCD",
            Self::Tlv2Led => "UNI FAN TL Wireless",
            Self::SlInf => "UNI FAN SL-INF Wireless",
            Self::Clv1 => "UNI FAN CL Wireless",
            Self::WaterBlock => "HydroShift II LCD-C (Wireless)",
            Self::WaterBlock2 => "HydroShift II LCD-S (Wireless)",
            Self::Strimer(_) => "Strimer Plus Wireless",
            Self::Lc217 => "Lancool 217 Wireless",
            Self::Led88 => "Universal Screen 8.8\" Wireless",
            Self::V150 => "Lancool V150 Wireless",
            Self::Unknown => "Wireless Fan",
        }
    }

    pub fn leds_per_fan(self) -> u8 {
        match self {
            Self::Tlv2Lcd | Self::Tlv2Led => 26,
            Self::Slv3Led | Self::Slv3Lcd => 40,
            Self::SlInf => 44,
            Self::Clv1 | Self::WaterBlock | Self::WaterBlock2 => 24,
            Self::Strimer(_) | Self::Lc217 | Self::Led88 | Self::V150 => 0,
            Self::Unknown => 20,
        }
    }

    pub fn supports_hw_mobo_sync(self) -> bool {
        matches!(self, Self::Slv3Led | Self::Slv3Lcd)
    }

    pub fn supports_mb_rgb_sync(self) -> bool {
        matches!(
            self,
            Self::Slv3Led | Self::Slv3Lcd | Self::Tlv2Lcd | Self::Tlv2Led
                | Self::SlInf | Self::Clv1 | Self::WaterBlock | Self::WaterBlock2
        )
    }

    pub fn is_aio(self) -> bool {
        matches!(self, Self::WaterBlock | Self::WaterBlock2)
    }

    pub fn is_rgb_only(self) -> bool {
        matches!(self, Self::Strimer(_) | Self::Lc217 | Self::Led88)
    }

    pub fn pump_led_count(self) -> u8 {
        if self.is_aio() {
            24
        } else {
            0
        }
    }

    pub fn pump_rpm_range(self) -> Option<(u32, u32)> {
        match self {
            Self::WaterBlock => Some((1600, 2500)),
            Self::WaterBlock2 => Some((1600, 3200)),
            _ => None,
        }
    }

    pub fn total_led_count_override(self) -> Option<u16> {
        match self {
            Self::Strimer(dt) => Some(match dt {
                1 => 116,
                2 => 132,
                3 => 174,
                _ => 88,
            }),
            Self::Lc217 => Some(96),
            Self::Led88 => Some(88),
            Self::V150 => Some(88),
            _ => None,
        }
    }

    /// Classify fan type from the fan-type byte in the device record.
    pub(super) fn from_fan_type_byte(b: u8) -> Self {
        match b {
            20..=23 => Self::Slv3Led,
            24..=26 => Self::Slv3Lcd,
            27 | 32..=35 => Self::Tlv2Lcd,
            28..=31 => Self::Tlv2Led,
            36..=39 => Self::SlInf,
            40 => Self::Clv1,
            41..=42 => Self::Clv1,
            _ => Self::Unknown,
        }
    }
}
