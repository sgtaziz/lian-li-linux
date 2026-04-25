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
        name: "HydroShift II LCD Square (Desktop Mode)",
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
];

impl DeviceFamily {
    pub fn has_lcd(self) -> bool {
        matches!(
            self,
            Self::TlLcd
                | Self::Slv3Lcd
                | Self::Tlv2Lcd
                | Self::HydroShiftLcd
                | Self::Galahad2Lcd
                | Self::HydroShift2Lcd
                | Self::Lancool207
                | Self::UniversalScreen
        )
    }

    pub fn has_fan(self) -> bool {
        // Wireless fan families (Slv3*, Tlv2*, SlInf, Clv1) are excluded here
        // because fan control goes through the wireless dongle, not USB.
        // Wireless-discovered devices get has_fan set explicitly in service.rs.
        matches!(
            self,
            Self::Ene6k77
                | Self::TlFan
                | Self::Galahad2Trinity
                | Self::HydroShiftLcd
                | Self::Galahad2Lcd
        )
    }

    pub fn has_pump(self) -> bool {
        matches!(
            self,
            Self::Galahad2Trinity
                | Self::HydroShiftLcd
                | Self::Galahad2Lcd
                | Self::HydroShift2Lcd
                | Self::WirelessAio
        )
    }

    pub fn has_rgb(self) -> bool {
        // Only wired HID devices that do RGB directly.
        // Wireless fans (Slv3, Tlv2, SlInf, Clv1) control RGB via the RF dongle,
        // not their USB connection — they get has_rgb from the wireless device entry.
        matches!(
            self,
            Self::Ene6k77
                | Self::TlFan
                | Self::Galahad2Trinity
                | Self::HydroShiftLcd
                | Self::Galahad2Lcd
                | Self::UniversalScreenLighting
        )
    }

    /// Whether this device is in desktop/display mode (CH340 firmware).
    /// These devices can be switched back to LCD mode via a button in the GUI.
    pub fn is_desktop_mode(self) -> bool {
        matches!(
            self,
            Self::HydroShift2LcdDesktop | Self::Lancool207Desktop | Self::UniversalScreenDesktop
        )
    }

    /// Whether this device supports switching to desktop mode.
    /// Only WinUSB LCD devices that have a CH340 display-mode counterpart.
    pub fn supports_display_mode_switch(self) -> bool {
        matches!(
            self,
            Self::HydroShift2Lcd | Self::Lancool207 | Self::UniversalScreen
        ) || self.is_desktop_mode()
    }
}

/// Look up a device family by VID/PID.
pub fn lookup_device(vid: u16, pid: u16) -> Option<&'static DeviceEntry> {
    KNOWN_DEVICES
        .iter()
        .find(|entry| entry.id.vid == vid && entry.id.pid == pid)
}

/// Returns true if this device family uses HID transport.
pub fn uses_hid(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::Ene6k77
            | DeviceFamily::TlFan
            | DeviceFamily::TlLcd
            | DeviceFamily::Galahad2Trinity
            | DeviceFamily::HydroShiftLcd
            | DeviceFamily::Galahad2Lcd
    )
}

/// Returns true if this device family uses USB bulk transport.
pub fn uses_usb_bulk(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::WirelessTx
            | DeviceFamily::WirelessRx
            | DeviceFamily::Slv3Lcd
            | DeviceFamily::Tlv2Lcd
            | DeviceFamily::HydroShift2Lcd
            | DeviceFamily::Lancool207
            | DeviceFamily::UniversalScreen
            | DeviceFamily::UniversalScreenLighting
    )
}
