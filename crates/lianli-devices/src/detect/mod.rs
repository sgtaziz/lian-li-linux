//! Device discovery and HID/USB backend opening.

mod backends;
mod binding;
mod controllers;
mod enumerate;

pub use backends::{
    hidraw_path_for_usb_topology, open_hid_lcd_by_topology, open_hid_lcd_by_vid_pid,
    open_hid_lcd_device, open_hid_transient, open_hid_transient_for_device, open_shared_hid,
    open_usb_bulk_backend,
};
pub use binding::ensure_hid_devices_bound;
pub use controllers::create_hid_lcd_device;
pub use enumerate::{
    enumerate_devices, known_devices_missing_from_libusb, probe_tl_lcd_port_indices,
};

use lianli_shared::device_id::DeviceFamily;
use lianli_transport::UsbContext;
use rusb::Device;

/// A detected USB device with its identified family.
#[derive(Debug)]
pub struct DetectedDevice {
    pub device: Device<UsbContext>,
    pub family: DeviceFamily,
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    pub bus: u8,
    pub address: u8,
    pub serial: Option<String>,
    /// HID usage page filter from the device entry. When set, only the HID
    /// interface with this usage page should be opened.
    pub hid_usage_page: Option<u16>,
}

impl DetectedDevice {
    pub fn device_id(&self) -> String {
        wired_identity(&self.topology_key())
    }

    pub fn legacy_device_id(&self) -> String {
        legacy_identity(self.serial.as_deref(), &self.topology_key())
    }

    pub fn topology_key(&self) -> String {
        usb_topology(
            self.vid,
            self.pid,
            self.bus,
            self.address,
            &self.device.port_numbers().unwrap_or_default(),
        )
    }
}

pub(crate) fn usb_topology(vid: u16, pid: u16, bus: u8, address: u8, ports: &[u8]) -> String {
    let port = lianli_shared::device_id::usb_port_path(bus, address, ports);
    format!("{vid:04x}:{pid:04x}:{port}")
}

pub(crate) fn wired_identity(topology: &str) -> String {
    format!("hid:{topology}")
}

fn legacy_identity(serial: Option<&str>, topology: &str) -> String {
    match serial {
        Some(serial) if !is_non_unique_serial(serial) => format!("hid:{serial}"),
        _ => format!("hid:{topology}"),
    }
}

/// Known non-unique HID serial strings (chip manufacturer names, firmware
/// version markers, etc. — not actual per-device serials).
const NON_UNIQUE_SERIALS: &[&str] = &["Nuvoton"];

fn is_non_unique_serial(s: &str) -> bool {
    NON_UNIQUE_SERIALS.contains(&s) || s.starts_with("TL_LCDV")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wired_hubs_have_independent_stable_physical_ids() {
        let first = usb_topology(0x0cf2, 0xa102, 1, 5, &[2, 3]);
        let second = usb_topology(0x0cf2, 0xa102, 1, 6, &[2, 4]);
        let id = wired_identity(&first);
        assert_eq!(id, "hid:0cf2:a102:1-2.3");
        assert_ne!(id, wired_identity(&second));
        assert_eq!(first, usb_topology(0x0cf2, 0xa102, 1, 12, &[2, 3]));
        assert_ne!(
            id,
            wired_identity(&usb_topology(0x0416, 0x7371, 1, 5, &[2, 3]))
        );
    }

    #[test]
    fn legacy_ids_remain_available_for_migration() {
        assert_eq!(
            legacy_identity(Some("shared"), "0cf2:a102:1-2"),
            "hid:shared"
        );
        assert_eq!(
            legacy_identity(Some("Nuvoton"), "0cf2:a102:1-2"),
            "hid:0cf2:a102:1-2"
        );
        assert_eq!(
            legacy_identity(Some("TL_LCDV1"), "0416:abcd:1-2"),
            "hid:0416:abcd:1-2"
        );
    }
}
