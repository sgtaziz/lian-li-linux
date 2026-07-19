//! Stable runtime identifiers for opened devices.
//!
//! These types replace the ad-hoc string formatting/parsing that was previously
//! spread across the daemon (`"wireless:aa:bb:..."`, `"serial:port3"`, etc.).
//! `Display`/`FromStr` round-trip the legacy wire formats so the IPC schema and
//! config files stay compatible while callers migrate to the typed form.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A 6-byte EUI-48 MAC address used to identify wireless devices on the RF bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    #[inline]
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    #[inline]
    pub const fn bytes(&self) -> [u8; 6] {
        self.0
    }
}

impl From<[u8; 6]> for MacAddress {
    fn from(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Matches `crate::wireless::DiscoveredDevice::mac_str` (lowercase hex colons).
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl FromStr for MacAddress {
    type Err = MacParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let segments: Vec<&str> = s.split(':').collect();
        if segments.len() != 6 {
            return Err(MacParseError);
        }
        let mut bytes = [0u8; 6];
        for (i, seg) in segments.iter().enumerate() {
            bytes[i] = u8::from_str_radix(seg, 16).map_err(|_| MacParseError)?;
        }
        Ok(Self(bytes))
    }
}

/// Error returned when a MAC address string cannot be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid MAC address (expected 6 lowercase hex octets separated by ':')")]
pub struct MacParseError;

/// Per-port / per-group suffix for wired devices that expose multiple logical
/// sub-devices through one USB serial number.
///
/// Examples:
/// - TL Fan controller has 4 fan ports → one `FanDevice` per port with
///   `WiredSuffix::Port(0..4)`.
/// - ENE 6K77 controller has 4 fan groups → one `RgbDevice` per group with
///   `WiredSuffix::Group(0..4)`.
/// - Single-zone devices (Galahad2 Trinity, LCDs, …) use `WiredSuffix::Unit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WiredSuffix {
    /// No suffix — the device is the only logical sub-device on its USB serial.
    Unit,
    /// TL Fan port index (0..4).
    Port(u8),
    /// ENE 6K77 fan-group index (0..4).
    Group(u8),
}

impl Default for WiredSuffix {
    fn default() -> Self {
        Self::Unit
    }
}

impl fmt::Display for WiredSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => Ok(()),
            Self::Port(n) => write!(f, ":port{}", n),
            Self::Group(n) => write!(f, ":group{}", n),
        }
    }
}

/// Stable identifier for an opened device, independent of transport.
///
/// Wire formats (round-trip with `Display`/`FromStr`):
/// - Wired: `"{serial}"` (Unit) | `"{serial}:port{n}"` (Port) | `"{serial}:group{n}"` (Group)
/// - Wireless, bound: `"wireless:{mac}"`
/// - Wireless, discovered but not yet paired: `"wireless-unbound:{mac}"`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceId {
    /// Wired (USB) device, optionally with a sub-device suffix.
    Wired {
        /// USB iSerialNumber string (rusb's `read_serial_number_string_ascii`).
        serial: String,
        /// Sub-device selector for multi-port / multi-group controllers.
        suffix: WiredSuffix,
    },
    /// Wireless device currently paired with the RF dongle.
    Wireless(MacAddress),
    /// Wireless device seen by the RX dongle but not yet paired/bound.
    WirelessUnbound(MacAddress),
}

impl DeviceId {
    #[inline]
    pub fn wired(serial: impl Into<String>) -> Self {
        Self::Wired {
            serial: serial.into(),
            suffix: WiredSuffix::Unit,
        }
    }

    #[inline]
    pub fn wired_port(serial: impl Into<String>, port: u8) -> Self {
        Self::Wired {
            serial: serial.into(),
            suffix: WiredSuffix::Port(port),
        }
    }

    #[inline]
    pub fn wired_group(serial: impl Into<String>, group: u8) -> Self {
        Self::Wired {
            serial: serial.into(),
            suffix: WiredSuffix::Group(group),
        }
    }

    #[inline]
    pub fn wireless(mac: MacAddress) -> Self {
        Self::Wireless(mac)
    }

    #[inline]
    pub fn wireless_unbound(mac: MacAddress) -> Self {
        Self::WirelessUnbound(mac)
    }

    /// `true` if this is a wireless (bound or unbound) device.
    #[inline]
    pub fn is_wireless(&self) -> bool {
        matches!(self, Self::Wireless(_) | Self::WirelessUnbound(_))
    }

    /// `true` if this is a wired (USB) device.
    #[inline]
    pub fn is_wired(&self) -> bool {
        matches!(self, Self::Wired { .. })
    }

    /// The MAC address if this is a wireless device.
    #[inline]
    pub fn mac(&self) -> Option<MacAddress> {
        match self {
            Self::Wireless(mac) | Self::WirelessUnbound(mac) => Some(*mac),
            _ => None,
        }
    }

    /// `true` if this is a bound wireless device (not unbound).
    #[inline]
    pub fn is_wireless_bound(&self) -> bool {
        matches!(self, Self::Wireless(_))
    }

    /// The USB serial number if this is a wired device.
    #[inline]
    pub fn serial(&self) -> Option<&str> {
        match self {
            Self::Wired { serial, .. } => Some(serial),
            _ => None,
        }
    }

    /// The sub-device suffix if this is a wired device.
    #[inline]
    pub fn suffix(&self) -> Option<WiredSuffix> {
        match self {
            Self::Wired { suffix, .. } => Some(*suffix),
            _ => None,
        }
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wired { serial, suffix } => write!(f, "{}{}", serial, suffix),
            Self::Wireless(mac) => write!(f, "wireless:{}", mac),
            Self::WirelessUnbound(mac) => write!(f, "wireless-unbound:{}", mac),
        }
    }
}

impl FromStr for DeviceId {
    type Err = DeviceIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("wireless-unbound:") {
            return Ok(Self::WirelessUnbound(
                MacAddress::from_str(rest).map_err(DeviceIdParseError::invalid_mac)?,
            ));
        }
        if let Some(rest) = s.strip_prefix("wireless:") {
            return Ok(Self::Wireless(
                MacAddress::from_str(rest).map_err(DeviceIdParseError::invalid_mac)?,
            ));
        }
        // Wired forms. Use `rsplit_once` so a serial containing ':' would still
        // parse correctly (USB serials from rusb's ascii helper are colon-free,
        // but we stay defensive).
        if let Some((base, n)) = s.rsplit_once(":port") {
            let port: u8 = n.parse().map_err(DeviceIdParseError::invalid_suffix)?;
            return Ok(Self::wired_port(base, port));
        }
        if let Some((base, n)) = s.rsplit_once(":group") {
            let group: u8 = n.parse().map_err(DeviceIdParseError::invalid_suffix)?;
            return Ok(Self::wired_group(base, group));
        }
        Ok(Self::wired(s))
    }
}

/// Error returned when a `DeviceId` string cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceIdParseError {
    #[error("invalid MAC address segment: {0}")]
    InvalidMac(#[from] MacParseError),
    #[error("invalid wired suffix (expected ':port<N>' or ':group<N>' with N in 0..=255)")]
    InvalidSuffix,
}

impl DeviceIdParseError {
    fn invalid_mac(_: MacParseError) -> Self {
        Self::InvalidMac(MacParseError)
    }
    fn invalid_suffix(_: std::num::ParseIntError) -> Self {
        Self::InvalidSuffix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_round_trips() {
        let mac = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let s = mac.to_string();
        assert_eq!(s, "aa:bb:cc:dd:ee:ff");
        let parsed: MacAddress = s.parse().unwrap();
        assert_eq!(mac, parsed);
    }

    #[test]
    fn mac_rejects_garbage() {
        assert!("aa-bb-cc-dd-ee-ff".parse::<MacAddress>().is_err());
        assert!("aa:bb:cc".parse::<MacAddress>().is_err());
        assert!((":".to_string()).parse::<MacAddress>().is_err());
        assert!("".parse::<MacAddress>().is_err());
    }

    #[test]
    fn wired_unit_round_trips() {
        let id = DeviceId::wired("ABC123");
        assert_eq!(id.to_string(), "ABC123");
        let parsed: DeviceId = "ABC123".parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn wired_port_round_trips() {
        let id = DeviceId::wired_port("ABC123", 2);
        assert_eq!(id.to_string(), "ABC123:port2");
        let parsed: DeviceId = "ABC123:port2".parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn wired_group_round_trips() {
        let id = DeviceId::wired_group("XYZ", 0);
        assert_eq!(id.to_string(), "XYZ:group0");
        let parsed: DeviceId = "XYZ:group0".parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn wireless_bound_round_trips() {
        let mac = MacAddress::new([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        let id = DeviceId::wireless(mac);
        assert_eq!(id.to_string(), "wireless:01:02:03:04:05:06");
        let parsed: DeviceId = "wireless:01:02:03:04:05:06".parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn wireless_unbound_round_trips() {
        let mac = MacAddress::new([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11]);
        let id = DeviceId::wireless_unbound(mac);
        assert_eq!(id.to_string(), "wireless-unbound:de:ad:be:ef:00:11");
        let parsed: DeviceId = "wireless-unbound:de:ad:be:ef:00:11".parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn wireless_unbound_does_not_match_bound_prefix() {
        // The parser must check `wireless-unbound:` before `wireless:` so a
        // bound-ID parser doesn't accidentally swallow the `-unbound:` tail.
        let s = "wireless-unbound:01:02:03:04:05:06";
        let parsed: DeviceId = s.parse().unwrap();
        assert!(matches!(parsed, DeviceId::WirelessUnbound(_)));
        assert!(!parsed.is_wireless_bound());
    }

    #[test]
    fn predicates_work() {
        let wired = DeviceId::wired_port("S", 1);
        let bound = DeviceId::wireless(MacAddress::new([0; 6]));
        let unbound = DeviceId::wireless_unbound(MacAddress::new([0; 6]));

        assert!(wired.is_wired());
        assert!(!wired.is_wireless());
        assert_eq!(wired.serial(), Some("S"));
        assert_eq!(wired.suffix(), Some(WiredSuffix::Port(1)));
        assert_eq!(wired.mac(), None);

        assert!(bound.is_wireless());
        assert!(bound.is_wireless_bound());
        assert_eq!(bound.mac(), Some(MacAddress::new([0; 6])));
        assert!(bound.serial().is_none());

        assert!(unbound.is_wireless());
        assert!(!unbound.is_wireless_bound());
    }

    #[test]
    fn invalid_suffix_rejected() {
        assert!("S:portX".parse::<DeviceId>().is_err());
        assert!("S:group-1".parse::<DeviceId>().is_err());
        assert!("S:port".parse::<DeviceId>().is_err());
        assert!("S:group".parse::<DeviceId>().is_err());
    }
}
