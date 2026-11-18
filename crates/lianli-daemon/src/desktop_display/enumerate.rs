use super::DeviceKey;
use anyhow::{Context, Result};
use lianli_devices::turzx;

/// A single detected TURZX device from a bus scan.
#[derive(Debug, Clone)]
pub struct TurzxDeviceMatch {
    pub device: rusb::Device<lianli_transport::UsbContext>,
    pub pid: u16,
    pub key: DeviceKey,
    pub panel_key: String,
}

/// Enumerate currently-attached TURZX panels on the USB bus.
pub fn enumerate_turzx() -> Result<Vec<TurzxDeviceMatch>> {
    let devices = lianli_transport::usb_context::devices().context("rusb::devices")?;
    let mut out = Vec::new();
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let vid = desc.vendor_id();
        let pid = desc.product_id();
        if !turzx::is_turzx(vid, pid) {
            continue;
        }
        let port = lianli_shared::device_id::usb_port_path(
            device.bus_number(),
            device.address(),
            &device.port_numbers().unwrap_or_default(),
        );
        out.push(TurzxDeviceMatch {
            pid,
            key: (device.bus_number(), device.address()),
            panel_key: lianli_shared::device_id::panel_key_for_port(&port),
            device,
        });
    }
    Ok(out)
}
