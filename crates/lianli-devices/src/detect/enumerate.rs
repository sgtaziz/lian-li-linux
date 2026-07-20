use super::DetectedDevice;
use anyhow::Result;
use lianli_shared::device_id::{DeviceFamily, UsbId, KNOWN_DEVICES};
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Enumerate all Lian Li USB devices on the system, sorted by (bus, address).
pub fn enumerate_devices() -> Result<Vec<DetectedDevice>> {
    let usb_devices = rusb::devices()?;
    let mut found = Vec::new();

    for device in usb_devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        let vid = desc.vendor_id();
        let pid = desc.product_id();
        let id = UsbId::new(vid, pid);

        if let Some(entry) = KNOWN_DEVICES.iter().find(|e| e.id == id) {
            let bus = device.bus_number();
            let address = device.address();

            let serial = device
                .open()
                .ok()
                .and_then(|h| h.read_serial_number_string_ascii(&desc).ok());

            debug!(
                "Found {} ({:04x}:{:04x}) at bus {} addr {} serial={}",
                entry.name,
                vid,
                pid,
                bus,
                address,
                serial.as_deref().unwrap_or("none")
            );

            found.push(DetectedDevice {
                device,
                family: entry.family,
                name: entry.name,
                vid,
                pid,
                bus,
                address,
                serial,
                hid_usage_page: entry.hid_usage_page,
            });
        }
    }

    found.sort_by_key(|d| (d.bus, d.address));
    Ok(found)
}

/// Probe TL LCD identities via rusb. The TL LCD firmware reports the same
/// iSerial across daisy-chained devices, so they cannot be told apart by
/// serial alone — we open each one and read its `(port, index)` identity
/// record to disambiguate.
pub fn probe_tl_lcd_port_indices_rusb(devices: &[DetectedDevice]) -> HashMap<String, (u8, u8)> {
    let mut out = HashMap::new();
    for det in devices.iter().filter(|d| d.family == DeviceFamily::TlLcd) {
        let transport = match RusbHid::open_by_usage(det.device.clone(), det.hid_usage_page) {
            Ok(t) => t,
            Err(e) => {
                warn!("TL LCD rusb open failed for {}: {e:#}", det.device_id());
                continue;
            }
        };
        let backend = Arc::new(Mutex::new(transport));
        let tl = crate::tl_lcd::TlLcdDevice::new(backend);
        match tl.read_identity_raw() {
            Ok(ident) => {
                out.insert(det.device_id(), (ident.port, ident.index));
            }
            Err(e) => warn!("TL LCD identity read failed for {}: {e:#}", det.device_id()),
        }
    }
    out
}

/// Find the rusb `Device` matching a VID/PID pair.
pub(super) fn find_usb_device(vid: u16, pid: u16) -> Option<rusb::Device<GlobalContext>> {
    rusb::devices().ok()?.iter().find(|d| {
        d.device_descriptor()
            .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
            .unwrap_or(false)
    })
}

/// Re-export so callers can refer to the rusb context type without depending
/// on rusb directly.
pub use rusb::GlobalContext;
