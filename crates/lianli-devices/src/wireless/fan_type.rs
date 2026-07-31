/// Wireless fan device type, determines minimum duty and RPM curves.
///
/// Byte ranges for classifying fan type:
/// ```text
/// SLV3   (base 20): 20-26  (LED: 20-22, LCD: 23-26)
/// TLV2   (base 27): 27-35  (LCD: 27,32-35, LED: 28-31)
/// SLINF  (base 36): 36-39  (LED only)
/// RL120:            40
/// CLV1:             41-42
/// SLINFV3(base 43): 43-50  (LCD when byte ∈ {43,44,47,48}, 44 LEDs/fan)
/// TLV3   (base 51): 51-58  (LCD when byte ∈ {51,52,55,56}, 26 LEDs/fan)
/// SLV4   (base 59): 59-62  (52 LEDs/fan)
/// P28V2:            63     (9 LEDs/fan, 3-bit gear field)
/// CLV2:             126-127 (24 LEDs/fan; byte 127 = reverse mount)
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
    /// SL-INF V3 wireless fans (44 LEDs/fan). LCD variant pairs with the
    /// SL Infinity Flex LCD wired endpoint. — 10% minimum duty.
    SlInfV3 { lcd: bool },
    /// TL V3 wireless fans (26 LEDs/fan). LCD variant pairs with the
    /// TL Flex LCD wired endpoint. — 11% minimum duty.
    TlV3 { lcd: bool },
    /// SL V4 wireless fans (52 LEDs/fan) — 14% minimum duty.
    SlV4,
    /// P28 V2 wireless fans (9 LEDs/fan, 3-bit gear field decoded elsewhere)
    /// — 8% minimum duty.
    P28V2,
    /// CL V2 wireless fans (24 LEDs/fan; byte 127 marks a reversed mount).
    /// Same PWM filter as `Clv1`. — 10% minimum duty.
    ClV2 { reverse: bool },
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
            Self::Slv3Led | Self::Slv3Lcd | Self::SlV4 => 14,
            Self::Tlv2Lcd
            | Self::Clv1
            | Self::ClV2 { .. }
            | Self::SlInfV3 { .. }
            | Self::WaterBlock
            | Self::WaterBlock2
            | Self::V150 => 10,
            Self::Tlv2Led | Self::SlInf | Self::TlV3 { .. } => 11,
            Self::P28V2 => 8,
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
            Self::SlInfV3 { lcd: false } => "UNI FAN SL-INF V3 Wireless",
            Self::SlInfV3 { lcd: true } => "UNI FAN SL-INF V3 Wireless LCD",
            Self::TlV3 { lcd: false } => "UNI FAN TL V3 Wireless",
            Self::TlV3 { lcd: true } => "UNI FAN TL V3 Wireless LCD",
            Self::SlV4 => "UNI FAN SL V4 Wireless",
            Self::P28V2 => "UNI FAN P28 V2 Wireless",
            Self::Clv1 => "UNI FAN CL Wireless",
            Self::ClV2 { .. } => "UNI FAN CL V2 Wireless",
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
            Self::Tlv2Lcd | Self::Tlv2Led | Self::TlV3 { .. } => 26,
            Self::Slv3Led | Self::Slv3Lcd => 40,
            Self::SlInf | Self::SlInfV3 { .. } => 44,
            Self::SlV4 => 52,
            Self::Clv1 | Self::ClV2 { .. } | Self::WaterBlock | Self::WaterBlock2 => 24,
            Self::P28V2 => 9,
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
            Self::Slv3Led
                | Self::Slv3Lcd
                | Self::Tlv2Lcd
                | Self::Tlv2Led
                | Self::SlInf
                | Self::SlInfV3 { .. }
                | Self::TlV3 { .. }
                | Self::SlV4
                | Self::P28V2
                | Self::Clv1
                | Self::ClV2 { .. }
                | Self::WaterBlock
                | Self::WaterBlock2
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
            20..=22 => Self::Slv3Led,
            23..=26 => Self::Slv3Lcd,
            27 | 32..=35 => Self::Tlv2Lcd,
            28..=31 => Self::Tlv2Led,
            36..=39 => Self::SlInf,
            40 | 41..=42 => Self::Clv1,
            43..=50 => Self::SlInfV3 {
                lcd: matches!(b, 43 | 44 | 47 | 48),
            },
            51..=58 => Self::TlV3 {
                lcd: matches!(b, 51 | 52 | 55 | 56),
            },
            59..=62 => Self::SlV4,
            63 => Self::P28V2,
            126 => Self::ClV2 { reverse: false },
            127 => Self::ClV2 { reverse: true },
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slinfv3_byte_classification() {
        for lcd_byte in [43, 44, 47, 48] {
            assert_eq!(
                WirelessFanType::from_fan_type_byte(lcd_byte),
                WirelessFanType::SlInfV3 { lcd: true },
                "byte {lcd_byte} should be SL-INF V3 LCD"
            );
        }
        for led_byte in [45, 46, 49, 50] {
            assert_eq!(
                WirelessFanType::from_fan_type_byte(led_byte),
                WirelessFanType::SlInfV3 { lcd: false },
                "byte {led_byte} should be SL-INF V3 LED"
            );
        }
        assert_eq!(WirelessFanType::SlInfV3 { lcd: true }.leds_per_fan(), 44);
        assert_eq!(
            WirelessFanType::SlInfV3 { lcd: false }.min_duty_percent(),
            10
        );
        assert!(WirelessFanType::SlInfV3 { lcd: true }.supports_mb_rgb_sync());
    }

    #[test]
    fn tlv3_byte_classification() {
        for lcd_byte in [51, 52, 55, 56] {
            assert_eq!(
                WirelessFanType::from_fan_type_byte(lcd_byte),
                WirelessFanType::TlV3 { lcd: true },
                "byte {lcd_byte} should be TL V3 LCD"
            );
        }
        for led_byte in [53, 54, 57, 58] {
            assert_eq!(
                WirelessFanType::from_fan_type_byte(led_byte),
                WirelessFanType::TlV3 { lcd: false },
                "byte {led_byte} should be TL V3 LED"
            );
        }
        assert_eq!(WirelessFanType::TlV3 { lcd: false }.leds_per_fan(), 26);
        assert_eq!(WirelessFanType::TlV3 { lcd: true }.min_duty_percent(), 11);
        assert!(WirelessFanType::TlV3 { lcd: false }.supports_mb_rgb_sync());
    }

    #[test]
    fn slv4_classification() {
        for b in 59..=62 {
            assert_eq!(
                WirelessFanType::from_fan_type_byte(b),
                WirelessFanType::SlV4,
                "byte {b} should be SL V4"
            );
        }
        assert_eq!(WirelessFanType::SlV4.leds_per_fan(), 52);
        assert_eq!(WirelessFanType::SlV4.min_duty_percent(), 14);
        assert!(WirelessFanType::SlV4.supports_mb_rgb_sync());
    }

    #[test]
    fn p28v2_classification() {
        assert_eq!(
            WirelessFanType::from_fan_type_byte(63),
            WirelessFanType::P28V2
        );
        assert_eq!(WirelessFanType::P28V2.leds_per_fan(), 9);
        assert_eq!(WirelessFanType::P28V2.min_duty_percent(), 8);
        assert!(WirelessFanType::P28V2.supports_mb_rgb_sync());
    }

    #[test]
    fn clv2_byte_classification() {
        assert_eq!(
            WirelessFanType::from_fan_type_byte(126),
            WirelessFanType::ClV2 { reverse: false }
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(127),
            WirelessFanType::ClV2 { reverse: true }
        );
        assert_eq!(WirelessFanType::ClV2 { reverse: false }.leds_per_fan(), 24);
        assert_eq!(
            WirelessFanType::ClV2 { reverse: true }.min_duty_percent(),
            10
        );
        assert!(WirelessFanType::ClV2 { reverse: false }.supports_mb_rgb_sync());
    }

    #[test]
    fn legacy_byte_ranges_preserved() {
        assert_eq!(
            WirelessFanType::from_fan_type_byte(20),
            WirelessFanType::Slv3Led
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(22),
            WirelessFanType::Slv3Led
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(23),
            WirelessFanType::Slv3Lcd
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(24),
            WirelessFanType::Slv3Lcd
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(28),
            WirelessFanType::Tlv2Led
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(36),
            WirelessFanType::SlInf
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(40),
            WirelessFanType::Clv1
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(42),
            WirelessFanType::Clv1
        );
        assert_eq!(
            WirelessFanType::from_fan_type_byte(255),
            WirelessFanType::Unknown
        );
    }

    #[test]
    fn display_names_distinct() {
        let names = [
            WirelessFanType::SlInfV3 { lcd: true }.display_name(),
            WirelessFanType::SlInfV3 { lcd: false }.display_name(),
            WirelessFanType::TlV3 { lcd: true }.display_name(),
            WirelessFanType::TlV3 { lcd: false }.display_name(),
            WirelessFanType::SlV4.display_name(),
            WirelessFanType::P28V2.display_name(),
            WirelessFanType::ClV2 { reverse: false }.display_name(),
        ];
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                assert_ne!(names[i], names[j], "duplicate display name");
            }
        }
    }
}
