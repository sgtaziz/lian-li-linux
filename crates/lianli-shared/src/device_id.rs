use serde::{Deserialize, Serialize};

/// All supported Lian Li device families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceFamily {
    /// ENE 6K77 wired fans (SL/AL series) — fan speed via HID
    Ene6k77,
    /// TL Fan controller — fan speed + handshake via HID
    TlFan,
    /// TLLCD — 400x400 LCD via HID
    TlLcd,
    /// Galahad2 Trinity AIO — pump + fan PWM via HID
    Galahad2Trinity,
    /// HydroShift LCD AIO — pump + fan + LCD 480x480 via HID
    HydroShiftLcd,
    /// Galahad2 LCD/Vision AIO — pump + fan + LCD (same HydroShift protocol)
    Galahad2Lcd,
    /// Wireless TX dongle
    WirelessTx,
    /// Wireless RX dongle
    WirelessRx,
    /// SLV3 wireless LCD (0x1CBE:0x0005)
    Slv3Lcd,
    /// SLV3 wireless LED fan (no LCD)
    Slv3Led,
    /// TLV2 wireless LCD (0x1CBE:0x0006)
    Tlv2Lcd,
    /// TLV2 wireless LED fan (no LCD)
    Tlv2Led,
    /// SL-INF wireless fan
    SlInf,
    /// CL / RL120 wireless fan
    Clv1,
    /// HydroShift II LCD Circle AIO — WinUSB 480x480
    HydroShift2Lcd,
    /// Lancool 207 Digital — WinUSB 1472x720
    Lancool207,
    /// Universal Screen 8.8" — WinUSB 1920x480
    UniversalScreen,
    /// HydroShift II LCD in desktop mode (CH340, 0x1A86:0xAD20)
    HydroShift2LcdDesktop,
    /// Lancool 207 in desktop mode (CH340, 0x1A86:0xACD1 / 0xAD11)
    Lancool207Desktop,
    /// Universal Screen 8.8" in desktop mode (CH340, 0x1A86:0xACE1 / 0xAD21)
    UniversalScreenDesktop,
    /// Wireless AIO (WaterBlock/WaterBlock2) — pump + fans via RF dongle
    WirelessAio,
    /// Wireless Strimer LED strip — RGB only via RF dongle
    WirelessStrimer,
    /// Wireless Lancool 217 case RGB ring — RGB only via RF dongle
    WirelessLc217,
    /// Wireless Universal Screen 8.8" LED ring — RGB only via RF dongle
    WirelessLed88,
    /// Wireless Lancool V150 case fan/RGB controller — dual-zone front/rear
    WirelessV150,
    /// Strimer Plus wired LED strip (0x0CF2:0xA200) — RGB only via HID
    StrimerPlus,
    /// Universal Screen 8.8" LED Ring — HID RGB controller (0x0416:0x8050)
    UniversalScreenLighting,
    /// Vision 9.2" LCD panel — WinUSB 464x1920
    Vision9p2,
    /// Vision 9.2" in desktop mode (CH340, 0x1A86:0xAD26)
    Vision9p2Desktop,
    /// TL Flex LCD — WinUSB fan-hub LCD (0x1CBE:0xA018)
    TlFlexLcd,
    /// SL Infinity Flex LCD — WinUSB fan-hub LCD (0x1CBE:0xA019)
    SlInfFlexLcd,
    /// Wired receiver controllers (P28 V2 0x43A8:0x0105, TL Flex 0x43A8:0x0101)
    WiredReceiver,
    /// HydroShift II OLED Curve — LCD MCU (0x1CBE:0xA068). LCD-only endpoint;
    /// speaks the DES-CBC encrypted 512-byte protocol. Owned by the LCD layer.
    HydroShift2OledCurveLcd,
    /// HydroShift II OLED Curve — LED MCU (0x0416:0x8051). Pump, coolant
    /// sensor (AIO), and the 45-LED RGB ring; speaks 8/64-byte plain bulk.
    HydroShift2OledCurveLed,
}

/// USB transport protocol a device uses on the wire.
///
/// Note: wireless-discovered devices (SLV3/TLV2/SL-INF/etc.) don't have a
/// transport of their own — they ride the RF dongle's USB bulk transport.
/// `DeviceFamily::transport_kind()` only describes the USB-attached controller
/// families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportKind {
    /// HID-over-USB (feature reports, output/input reports) via libusb.
    Hid,
    /// WinUSB-style USB bulk transfer.
    UsbBulk,
}

/// Capability bitflags for a `DeviceFamily`.
///
/// Hand-rolled (no `bitflags` dep) — small enough that a `u16` is plenty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct DeviceCapabilities(pub u16);

impl DeviceCapabilities {
    // Capability bits.
    pub const NONE: Self = Self(0);
    pub const FAN: Self = Self(1 << 0);
    pub const LCD: Self = Self(1 << 1);
    pub const RGB: Self = Self(1 << 2);
    pub const PUMP: Self = Self(1 << 3);
    pub const AIO: Self = Self(1 << 4);
    /// WinUSB LCD that has a CH340 desktop-mode counterpart.
    pub const DISPLAY_MODE_SWITCH: Self = Self(1 << 5);
    /// Currently in CH340 desktop/display mode (companion endpoints).
    pub const DESKTOP_MODE: Self = Self(1 << 6);
    /// Acts as the RF dongle (TX/RX) — special-cased by the wireless layer.
    pub const WIRELESS_DONGLE: Self = Self(1 << 7);

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Const-context bitwise OR (the `BitOr` trait impl is not `const`).
    ///
    /// Use this inside `const fn`; use `|` elsewhere.
    #[inline]
    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn insert(mut self, other: Self) -> Self {
        self.0 |= other.0;
        self
    }

    #[inline]
    pub const fn remove(mut self, other: Self) -> Self {
        self.0 &= !other.0;
        self
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub fn iter(self) -> DeviceCapabilityIter {
        DeviceCapabilityIter {
            remaining: self.0,
            emitted: 0,
        }
    }
}

impl core::ops::BitOr for DeviceCapabilities {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for DeviceCapabilities {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for DeviceCapabilities {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Iterator over the capability bits set in a `DeviceCapabilities`.
#[derive(Debug, Clone)]
pub struct DeviceCapabilityIter {
    remaining: u16,
    emitted: usize,
}

impl Iterator for DeviceCapabilityIter {
    type Item = DeviceCapabilities;
    fn next(&mut self) -> Option<Self::Item> {
        while self.emitted < 16 {
            let bit = 1u16 << self.emitted;
            self.emitted += 1;
            if (self.remaining & bit) != 0 {
                return Some(DeviceCapabilities(bit));
            }
        }
        None
    }
}

/// USB Vendor/Product ID pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsbId {
    pub vid: u16,
    pub pid: u16,
}

impl UsbId {
    pub const fn new(vid: u16, pid: u16) -> Self {
        Self { vid, pid }
    }
}

/// Known device entry: maps USB IDs to a device family.
#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub id: UsbId,
    pub family: DeviceFamily,
    pub name: &'static str,
    /// HID usage page filter. When set, only the HID interface with this
    /// usage page is used (devices often expose multiple HID interfaces).
    pub hid_usage_page: Option<u16>,
}

/// All known Lian Li USB device identifiers.
pub static KNOWN_DEVICES: &[DeviceEntry] = &[
    // Wireless dongles
    DeviceEntry {
        id: UsbId::new(0x0416, 0x8040),
        family: DeviceFamily::WirelessTx,
        name: "Wireless TX Dongle",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x8041),
        family: DeviceFamily::WirelessRx,
        name: "Wireless RX Dongle",
        hid_usage_page: None,
    },
    // Wireless V2 dongles (CH340 chipset, VID=0x1A86)
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xE304),
        family: DeviceFamily::WirelessTx,
        name: "Wireless TX Dongle V2",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xE305),
        family: DeviceFamily::WirelessRx,
        name: "Wireless RX Dongle V2",
        hid_usage_page: None,
    },
    // Universal Screen 8.8" LED Ring
    DeviceEntry {
        id: UsbId::new(0x0416, 0x8050),
        family: DeviceFamily::UniversalScreenLighting,
        name: "Universal Screen 8.8\" LED Ring",
        hid_usage_page: None,
    },
    // ENE 6K77 wired fans (SL/AL series)
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA100),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA101),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA102),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA103),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA104),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA105),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA106),
        family: DeviceFamily::Ene6k77,
        name: "ENE 6K77 SL/AL Fan Controller",
        hid_usage_page: None,
    },
    // Strimer Plus wired LED strip
    DeviceEntry {
        id: UsbId::new(0x0CF2, 0xA200),
        family: DeviceFamily::StrimerPlus,
        name: "Strimer Plus",
        hid_usage_page: None,
    },
    // TL Fan controller — usage page 0xFF1B selects the control interface
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7372),
        family: DeviceFamily::TlFan,
        name: "TL Fan Controller",
        hid_usage_page: Some(0xFF1B),
    },
    // TLLCD
    DeviceEntry {
        id: UsbId::new(0x04FC, 0x7393),
        family: DeviceFamily::TlLcd,
        name: "UNI FAN TL LCD",
        hid_usage_page: None,
    },
    // Galahad2 Trinity — usage page 0xFF1B selects the control interface
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7371),
        family: DeviceFamily::Galahad2Trinity,
        name: "Galahad II Trinity AIO",
        hid_usage_page: Some(0xFF1B),
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7373),
        family: DeviceFamily::Galahad2Trinity,
        name: "Galahad II Trinity AIO",
        hid_usage_page: Some(0xFF1B),
    },
    // HydroShift LCD
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7398),
        family: DeviceFamily::HydroShiftLcd,
        name: "HydroShift LCD AIO",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7399),
        family: DeviceFamily::HydroShiftLcd,
        name: "HydroShift LCD AIO",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x739A),
        family: DeviceFamily::HydroShiftLcd,
        name: "HydroShift LCD AIO",
        hid_usage_page: None,
    },
    // Galahad2 LCD / Vision
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7391),
        family: DeviceFamily::Galahad2Lcd,
        name: "Galahad II LCD AIO",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x7395),
        family: DeviceFamily::Galahad2Lcd,
        name: "Galahad II Vision AIO",
        hid_usage_page: None,
    },
    // USB bulk LCD devices (VID=0x1CBE)
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0x0005),
        family: DeviceFamily::Slv3Lcd,
        name: "SLV3 Wireless LCD Fan",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0x0006),
        family: DeviceFamily::Tlv2Lcd,
        name: "TLV2 Wireless LCD Fan",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA021),
        family: DeviceFamily::HydroShift2Lcd,
        name: "HydroShift II LCD Circle",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA034),
        family: DeviceFamily::HydroShift2Lcd,
        name: "HydroShift II LCD Square",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA065),
        family: DeviceFamily::Lancool207,
        name: "Lancool 207 Digital",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA088),
        family: DeviceFamily::UniversalScreen,
        name: "Universal Screen 8.8\"",
        hid_usage_page: None,
    },
    // Desktop-mode variants (CH340, VID=0x1A86) — same physical device as 0x1CBE LCD
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD20),
        family: DeviceFamily::HydroShift2LcdDesktop,
        name: "HydroShift II LCD Circle (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD22),
        family: DeviceFamily::HydroShift2LcdDesktop,
        name: "HydroShift II LCD Square (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD23),
        family: DeviceFamily::HydroShift2LcdDesktop,
        name: "HydroShift II OLED Curve (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xACD1),
        family: DeviceFamily::Lancool207Desktop,
        name: "Lancool 207 Digital (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD11),
        family: DeviceFamily::Lancool207Desktop,
        name: "Lancool 207 Digital (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xACE1),
        family: DeviceFamily::UniversalScreenDesktop,
        name: "Universal Screen 8.8\" (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD21),
        family: DeviceFamily::UniversalScreenDesktop,
        name: "Universal Screen 8.8\" (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA092),
        family: DeviceFamily::Vision9p2,
        name: "Vision 9.2\"",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1A86, 0xAD26),
        family: DeviceFamily::Vision9p2Desktop,
        name: "Vision 9.2\" (Desktop Mode)",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA018),
        family: DeviceFamily::TlFlexLcd,
        name: "TL Flex LCD",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA019),
        family: DeviceFamily::SlInfFlexLcd,
        name: "SL Infinity Flex LCD",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0101),
        family: DeviceFamily::WiredReceiver,
        name: "TL Flex Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0102),
        family: DeviceFamily::WiredReceiver,
        name: "TL Flex LCD Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0103),
        family: DeviceFamily::WiredReceiver,
        name: "SL-INF Flex Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0104),
        family: DeviceFamily::WiredReceiver,
        name: "SL-INF Flex LCD Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0105),
        family: DeviceFamily::WiredReceiver,
        name: "P28 V2 Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0106),
        family: DeviceFamily::WiredReceiver,
        name: "SL V4 Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x43A8, 0x0107),
        family: DeviceFamily::WiredReceiver,
        name: "CL V2 Controller",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x1CBE, 0xA068),
        family: DeviceFamily::HydroShift2OledCurveLcd,
        name: "HydroShift II OLED Curve",
        hid_usage_page: None,
    },
    DeviceEntry {
        id: UsbId::new(0x0416, 0x8051),
        family: DeviceFamily::HydroShift2OledCurveLed,
        name: "HydroShift II OLED Curve LED",
        hid_usage_page: None,
    },
];

impl DeviceFamily {
    /// Capability flags for this family — single source of truth.
    ///
    /// The legacy `has_*` / `is_*` / `supports_*` predicates below all delegate
    /// here, and new capability checks should query this directly:
    ///
    /// ```ignore
    /// if family.capabilities().contains(DeviceCapabilities::FAN) { ... }
    /// ```
    pub const fn capabilities(self) -> DeviceCapabilities {
        match self {
            // Wired HID fan controllers.
            Self::Ene6k77 => DeviceCapabilities::FAN.or(DeviceCapabilities::RGB),
            Self::TlFan => DeviceCapabilities::FAN.or(DeviceCapabilities::RGB),
            Self::StrimerPlus => DeviceCapabilities::RGB,

            // Wired HID AIOs (fans + pump + RGB; LCD on HydroShift/Galahad2 LCD).
            Self::Galahad2Trinity => DeviceCapabilities::FAN
                .or(DeviceCapabilities::PUMP)
                .or(DeviceCapabilities::AIO)
                .or(DeviceCapabilities::RGB),
            Self::HydroShiftLcd | Self::Galahad2Lcd => DeviceCapabilities::FAN
                .or(DeviceCapabilities::PUMP)
                .or(DeviceCapabilities::AIO)
                .or(DeviceCapabilities::RGB)
                .or(DeviceCapabilities::LCD),

            // Wired HID LCD (no pump).
            Self::TlLcd => DeviceCapabilities::LCD,

            // WinUSB bulk LCDs (support switching to CH340 desktop mode).
            Self::HydroShift2Lcd => DeviceCapabilities::LCD
                .or(DeviceCapabilities::FAN)
                .or(DeviceCapabilities::PUMP)
                .or(DeviceCapabilities::AIO)
                .or(DeviceCapabilities::RGB)
                .or(DeviceCapabilities::DISPLAY_MODE_SWITCH),
            Self::Lancool207 | Self::UniversalScreen | Self::Vision9p2 => {
                DeviceCapabilities::LCD.or(DeviceCapabilities::DISPLAY_MODE_SWITCH)
            }
            Self::TlFlexLcd | Self::SlInfFlexLcd => DeviceCapabilities::LCD,
            Self::WiredReceiver => DeviceCapabilities::FAN.or(DeviceCapabilities::RGB),
            Self::HydroShift2OledCurveLcd => {
                DeviceCapabilities::LCD.or(DeviceCapabilities::DISPLAY_MODE_SWITCH)
            }
            Self::HydroShift2OledCurveLed => DeviceCapabilities::PUMP
                .or(DeviceCapabilities::AIO)
                .or(DeviceCapabilities::RGB),

            // Desktop-mode companions (CH340 firmware) — same physical device
            // as the WinUSB LCDs above, currently in display mode.
            Self::HydroShift2LcdDesktop
            | Self::Lancool207Desktop
            | Self::UniversalScreenDesktop
            | Self::Vision9p2Desktop => DeviceCapabilities::LCD
                .or(DeviceCapabilities::DESKTOP_MODE)
                .or(DeviceCapabilities::DISPLAY_MODE_SWITCH),

            // Wired USB-bulk LED controllers.
            Self::UniversalScreenLighting => DeviceCapabilities::RGB,

            // RF dongles (TX/RX). Not user-visible devices themselves.
            Self::WirelessTx | Self::WirelessRx => DeviceCapabilities::WIRELESS_DONGLE,

            // Wireless-discovered devices. Their capability flags are populated
            // from `WirelessFanType` once the RX dongle reports a record; the
            // defaults below describe the family-level baseline.
            Self::Slv3Lcd | Self::Tlv2Lcd => DeviceCapabilities::FAN
                .or(DeviceCapabilities::LCD)
                .or(DeviceCapabilities::RGB),
            Self::Slv3Led | Self::Tlv2Led | Self::SlInf | Self::Clv1 => {
                DeviceCapabilities::FAN.or(DeviceCapabilities::RGB)
            }
            Self::WirelessAio => DeviceCapabilities::FAN
                .or(DeviceCapabilities::PUMP)
                .or(DeviceCapabilities::AIO)
                .or(DeviceCapabilities::RGB),
            Self::WirelessStrimer
            | Self::WirelessLc217
            | Self::WirelessLed88
            | Self::WirelessV150 => DeviceCapabilities::RGB,
        }
    }

    /// USB transport this family uses on the wire.
    ///
    /// Wireless-discovered families return `UsbBulk` because they ride the RF
    /// dongle's transport; the dongle itself is the only USB-attached device.
    pub const fn transport_kind(self) -> TransportKind {
        match self {
            // HID feature-report / output-report families.
            Self::Ene6k77
            | Self::TlFan
            | Self::TlLcd
            | Self::Galahad2Trinity
            | Self::HydroShiftLcd
            | Self::Galahad2Lcd
            | Self::StrimerPlus => TransportKind::Hid,

            // WinUSB-style bulk families (incl. RF dongles and desktop-mode
            // CH340 companions).
            Self::UniversalScreenLighting
            | Self::WirelessTx
            | Self::WirelessRx
            | Self::Slv3Lcd
            | Self::Tlv2Lcd
            | Self::HydroShift2Lcd
            | Self::Lancool207
            | Self::UniversalScreen
            | Self::Vision9p2
            | Self::TlFlexLcd
            | Self::SlInfFlexLcd
            | Self::HydroShift2LcdDesktop
            | Self::Lancool207Desktop
            | Self::UniversalScreenDesktop
            | Self::Vision9p2Desktop
            | Self::WiredReceiver
            | Self::HydroShift2OledCurveLcd
            | Self::HydroShift2OledCurveLed => TransportKind::UsbBulk,

            // Pure wireless-discovered (no USB identity of their own).
            Self::Slv3Led
            | Self::Tlv2Led
            | Self::SlInf
            | Self::Clv1
            | Self::WirelessAio
            | Self::WirelessStrimer
            | Self::WirelessLc217
            | Self::WirelessLed88
            | Self::WirelessV150 => TransportKind::UsbBulk,
        }
    }

    /// `true` if the family exposes an LCD screen.
    #[inline]
    pub fn has_lcd(self) -> bool {
        self.capabilities().contains(DeviceCapabilities::LCD)
    }

    /// `true` if the family controls fans directly over USB.
    ///
    /// Wireless fan families (`Slv3*`, `Tlv2*`, `SlInf`, `Clv1`) are included
    /// here — the daemon previously excluded them at the USB layer because fan
    /// control went through the dongle. With the unified `Device` trait (Crate
    /// 3), wireless devices implement `FanDevice` directly through dongle
    /// adapters, so the family capability is `true`.
    #[inline]
    pub fn has_fan(self) -> bool {
        self.capabilities().contains(DeviceCapabilities::FAN)
    }

    /// `true` if the family has a pump (AIOs).
    #[inline]
    pub fn has_pump(self) -> bool {
        self.capabilities().contains(DeviceCapabilities::PUMP)
    }

    /// `true` if the family is a full AIO (fans + pump + coolant sensor).
    #[inline]
    pub fn is_aio(self) -> bool {
        self.capabilities().contains(DeviceCapabilities::AIO)
    }

    /// `true` if the family exposes RGB control.
    #[inline]
    pub fn has_rgb(self) -> bool {
        self.capabilities().contains(DeviceCapabilities::RGB)
    }

    /// `true` if this is the RF dongle (TX/RX).
    #[inline]
    pub fn is_wireless_dongle(self) -> bool {
        self.capabilities()
            .contains(DeviceCapabilities::WIRELESS_DONGLE)
    }

    /// Whether this device is in desktop/display mode (CH340 firmware).
    /// These devices can be switched back to LCD mode via a button in the GUI.
    #[inline]
    pub fn is_desktop_mode(self) -> bool {
        self.capabilities()
            .contains(DeviceCapabilities::DESKTOP_MODE)
    }

    /// Whether this device supports switching to desktop mode.
    /// Only WinUSB LCD devices that have a CH340 display-mode counterpart.
    #[inline]
    pub fn supports_display_mode_switch(self) -> bool {
        self.capabilities()
            .contains(DeviceCapabilities::DISPLAY_MODE_SWITCH)
    }

    /// `true` if this family speaks HID-over-USB (libusb control/interrupt
    /// transfers for feature/input/output reports).
    #[inline]
    pub fn uses_hid(self) -> bool {
        matches!(self.transport_kind(), TransportKind::Hid)
    }

    /// `true` if this family uses WinUSB-style bulk transfers.
    #[inline]
    pub fn uses_usb_bulk(self) -> bool {
        matches!(self.transport_kind(), TransportKind::UsbBulk)
    }
}

/// Look up a device family by VID/PID.
pub fn lookup_device(vid: u16, pid: u16) -> Option<&'static DeviceEntry> {
    KNOWN_DEVICES
        .iter()
        .find(|entry| entry.id.vid == vid && entry.id.pid == pid)
}

/// Returns true if this device family uses HID transport.
///
/// Prefer `DeviceFamily::uses_hid()`; this free function is retained for
/// existing call sites and will be removed once the daemon migration completes.
#[inline]
pub fn uses_hid(family: DeviceFamily) -> bool {
    family.uses_hid()
}

/// Returns true if this device family uses USB bulk transport.
///
/// Prefer `DeviceFamily::uses_usb_bulk()`; this free function is retained for
/// existing call sites and will be removed once the daemon migration completes.
#[inline]
pub fn uses_usb_bulk(family: DeviceFamily) -> bool {
    family.uses_usb_bulk()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `has_*` predicates must agree with `capabilities()` so callers can
    /// migrate to the typed form without behaviour drift.
    #[test]
    fn has_predicates_match_capabilities() {
        for &family in ALL_FAMILIES {
            let caps = family.capabilities();
            assert_eq!(family.has_lcd(), caps.contains(DeviceCapabilities::LCD));
            assert_eq!(family.has_fan(), caps.contains(DeviceCapabilities::FAN));
            assert_eq!(family.has_pump(), caps.contains(DeviceCapabilities::PUMP));
            assert_eq!(family.has_rgb(), caps.contains(DeviceCapabilities::RGB));
            assert_eq!(family.is_aio(), caps.contains(DeviceCapabilities::AIO));
            assert_eq!(
                family.is_wireless_dongle(),
                caps.contains(DeviceCapabilities::WIRELESS_DONGLE)
            );
            assert_eq!(
                family.is_desktop_mode(),
                caps.contains(DeviceCapabilities::DESKTOP_MODE)
            );
            assert_eq!(
                family.supports_display_mode_switch(),
                caps.contains(DeviceCapabilities::DISPLAY_MODE_SWITCH)
            );
        }
    }

    #[test]
    fn transport_kind_is_exhaustive() {
        for &family in ALL_FAMILIES {
            // Every family must have a definite transport.
            let _ = family.transport_kind();
            // uses_hid / uses_usb_bulk are mutually exclusive and exhaustive.
            assert_eq!(family.uses_hid(), !family.uses_usb_bulk());
        }
    }

    #[test]
    fn wired_aio_families_have_full_stack() {
        for family in [
            DeviceFamily::Galahad2Trinity,
            DeviceFamily::HydroShiftLcd,
            DeviceFamily::Galahad2Lcd,
        ] {
            let caps = family.capabilities();
            assert!(caps.contains(DeviceCapabilities::FAN));
            assert!(caps.contains(DeviceCapabilities::PUMP));
            assert!(caps.contains(DeviceCapabilities::AIO));
            assert!(caps.contains(DeviceCapabilities::RGB));
            assert!(family.uses_hid());
        }
    }

    #[test]
    fn display_mode_switch_pairs_are_consistent() {
        // Each desktop-mode variant must also be display-mode-switch capable.
        for family in [
            DeviceFamily::HydroShift2LcdDesktop,
            DeviceFamily::Lancool207Desktop,
            DeviceFamily::UniversalScreenDesktop,
        ] {
            assert!(family.is_desktop_mode());
            assert!(family.supports_display_mode_switch());
        }
        // Each "LCD mode" counterpart must also be switch-capable.
        for family in [
            DeviceFamily::HydroShift2Lcd,
            DeviceFamily::Lancool207,
            DeviceFamily::UniversalScreen,
        ] {
            assert!(!family.is_desktop_mode());
            assert!(family.supports_display_mode_switch());
        }
    }

    #[test]
    fn wireless_dongles_are_dongles() {
        for family in [DeviceFamily::WirelessTx, DeviceFamily::WirelessRx] {
            assert!(family.is_wireless_dongle());
            assert!(!family.has_fan());
            assert!(!family.has_lcd());
            assert!(!family.has_rgb());
        }
    }

    #[test]
    fn free_function_shims_match_methods() {
        for &family in ALL_FAMILIES {
            assert_eq!(uses_hid(family), family.uses_hid());
            assert_eq!(uses_usb_bulk(family), family.uses_usb_bulk());
        }
    }

    /// Compile-time inventory of every `DeviceFamily` variant. Add new variants
    /// here; the tests above will then cover them automatically.
    const ALL_FAMILIES: &[DeviceFamily] = &[
        DeviceFamily::Ene6k77,
        DeviceFamily::TlFan,
        DeviceFamily::TlLcd,
        DeviceFamily::Galahad2Trinity,
        DeviceFamily::HydroShiftLcd,
        DeviceFamily::Galahad2Lcd,
        DeviceFamily::WirelessTx,
        DeviceFamily::WirelessRx,
        DeviceFamily::Slv3Lcd,
        DeviceFamily::Slv3Led,
        DeviceFamily::Tlv2Lcd,
        DeviceFamily::Tlv2Led,
        DeviceFamily::SlInf,
        DeviceFamily::Clv1,
        DeviceFamily::HydroShift2Lcd,
        DeviceFamily::Lancool207,
        DeviceFamily::UniversalScreen,
        DeviceFamily::HydroShift2LcdDesktop,
        DeviceFamily::Lancool207Desktop,
        DeviceFamily::UniversalScreenDesktop,
        DeviceFamily::WirelessAio,
        DeviceFamily::WirelessStrimer,
        DeviceFamily::WirelessLc217,
        DeviceFamily::WirelessLed88,
        DeviceFamily::WirelessV150,
        DeviceFamily::StrimerPlus,
        DeviceFamily::UniversalScreenLighting,
    ];
}
