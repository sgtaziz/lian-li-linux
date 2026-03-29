use crate::error::TransportError;
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;
use tracing::debug;

pub struct RusbHidTransport {
    handle: DeviceHandle<GlobalContext>,
    iface: u8,
    ep_in: u8,
    ep_out: Option<u8>,
}

impl RusbHidTransport {
    /// Open a HID interface, discovering it by usage page on the same handle.
    ///
    /// This combines interface discovery and opening into a single call that
    /// reuses one USB handle. Some devices (e.g. TL Fan) stop responding if
    /// the handle used for descriptor reads is dropped before a new handle
    /// claims the interface.
    pub fn open_by_usage(
        device: Device<GlobalContext>,
        usage_page: Option<u16>,
    ) -> Result<Self, TransportError> {
        let handle = device.open()?;
        let config = device.active_config_descriptor()?;

        // Collect HID interfaces
        let mut hid_ifaces: Vec<u8> = Vec::new();
        for iface in config.interfaces() {
            for desc in iface.descriptors() {
                if desc.class_code() == 0x03 {
                    hid_ifaces.push(desc.interface_number());
                }
            }
        }

        if hid_ifaces.is_empty() {
            return Err(TransportError::Other("No HID interfaces found".into()));
        }

        // Detach kernel drivers from all HID interfaces on this device
        for &iface_num in &hid_ifaces {
            if handle.kernel_driver_active(iface_num).unwrap_or(false) {
                let _ = handle.detach_kernel_driver(iface_num);
                debug!("RusbHid: detached kernel driver from interface {iface_num}");
            }
        }

        // Find the target interface by usage page
        let target_iface = if let Some(required_page) = usage_page {
            let mut matched = None;
            for &iface_num in &hid_ifaces {
                let mut buf = [0u8; 256];
                let result = handle.read_control(
                    0x81,
                    0x06,
                    (0x22u16) << 8,
                    iface_num as u16,
                    &mut buf,
                    Duration::from_millis(1000),
                );
                if let Ok(n) = result {
                    if let Some(page) = parse_usage_page(&buf[..n]) {
                        if page == required_page {
                            debug!(
                                "RusbHid: interface {iface_num} matches usage page {required_page:#06x}"
                            );
                            matched = Some(iface_num);
                            break;
                        }
                    }
                }
            }
            matched.unwrap_or_else(|| {
                debug!(
                    "RusbHid: no interface matched usage page {required_page:#06x}, using first"
                );
                hid_ifaces[0]
            })
        } else {
            hid_ifaces[0]
        };

        // Claim the target interface (same handle, no drop in between)
        handle.claim_interface(target_iface)?;

        // Find endpoints
        let mut ep_in: Option<u8> = None;
        let mut ep_out: Option<u8> = None;
        for iface_group in config.interfaces() {
            for desc in iface_group.descriptors() {
                if desc.interface_number() != target_iface {
                    continue;
                }
                for ep in desc.endpoint_descriptors() {
                    if ep.transfer_type() != rusb::TransferType::Interrupt {
                        continue;
                    }
                    match ep.direction() {
                        rusb::Direction::In => ep_in = ep_in.or(Some(ep.address())),
                        rusb::Direction::Out => ep_out = ep_out.or(Some(ep.address())),
                    }
                }
            }
        }

        let ep_in = ep_in.ok_or_else(|| {
            TransportError::Other("RusbHid: no interrupt IN endpoint found".into())
        })?;

        if ep_out.is_some() {
            debug!("RusbHid: interface={target_iface} ep_in=0x{ep_in:02x} ep_out=0x{:02x}", ep_out.unwrap());
        } else {
            debug!("RusbHid: interface={target_iface} ep_in=0x{ep_in:02x} (using SET_REPORT for writes)");
        }

        Ok(Self {
            handle,
            iface: target_iface,
            ep_in,
            ep_out,
        })
    }

    /// Perform a USB port reset on the device (USBDEVFS_RESET ioctl).
    /// This resets the device firmware state, which can fix malformed HID
    /// report descriptors on devices that persist bad state across reboots.
    ///
    /// Detaches kernel drivers from all HID interfaces before the reset and
    /// reattaches them afterwards so the kernel re-enumerates the device and
    /// creates fresh hidraw nodes.
    pub fn reset_usb_device(device: &Device<GlobalContext>) -> Result<(), TransportError> {
        let handle = device.open()?;

        // Collect HID interfaces to detach/reattach kernel drivers.
        let mut hid_ifaces: Vec<u8> = Vec::new();
        if let Ok(config) = device.active_config_descriptor() {
            for iface in config.interfaces() {
                for desc in iface.descriptors() {
                    if desc.class_code() == 0x03 {
                        hid_ifaces.push(desc.interface_number());
                    }
                }
            }
        }

        // Detach kernel drivers before reset.
        for &iface in &hid_ifaces {
            match handle.kernel_driver_active(iface) {
                Ok(true) => {
                    let _ = handle.detach_kernel_driver(iface);
                    debug!("reset_usb_device: detached kernel driver from interface {iface}");
                }
                _ => {}
            }
        }

        handle.reset().map_err(|e| {
            TransportError::Other(format!("USB device reset failed: {e}"))
        })?;

        // Reattach kernel drivers so the kernel re-enumerates HID descriptors.
        for &iface in &hid_ifaces {
            let _ = handle.attach_kernel_driver(iface);
            debug!("reset_usb_device: reattached kernel driver on interface {iface}");
        }

        Ok(())
    }

    pub fn send_feature_report(&self, data: &[u8]) -> Result<usize, TransportError> {
        let report_id = data.first().copied().unwrap_or(0) as u16;
        let w_value = (0x03u16 << 8) | report_id;
        let n = self.handle.write_control(
            0x21,
            0x09, // SET_REPORT
            w_value,
            self.iface as u16,
            data,
            Duration::from_millis(5000),
        )?;
        Ok(n)
    }

    pub fn get_feature_report(&self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let report_id = buf.first().copied().unwrap_or(0) as u16;
        let w_value = (0x03u16 << 8) | report_id;
        let n = self.handle.read_control(
            0xA1,
            0x01, // GET_REPORT
            w_value,
            self.iface as u16,
            buf,
            Duration::from_millis(5000),
        )?;
        Ok(n)
    }

    pub fn get_input_report(&self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let report_id = buf.first().copied().unwrap_or(0) as u16;
        let w_value = (0x01u16 << 8) | report_id;
        let n = self.handle.read_control(
            0xA1,
            0x01, // GET_REPORT
            w_value,
            self.iface as u16,
            buf,
            Duration::from_millis(5000),
        )?;
        Ok(n)
    }

    pub fn write(&self, data: &[u8]) -> Result<usize, TransportError> {
        if let Some(ep_out) = self.ep_out {
            let n = self
                .handle
                .write_interrupt(ep_out, data, Duration::from_millis(5000))?;
            Ok(n)
        } else {
            // SET_REPORT control transfer: report type = Output (0x02), report ID = data[0]
            let report_id = data.first().copied().unwrap_or(0) as u16;
            let report_type: u16 = 0x02;
            let w_value = (report_type << 8) | report_id;
            let n = self.handle.write_control(
                0x21, // Host-to-device, Class, Interface
                0x09, // SET_REPORT
                w_value,
                self.iface as u16,
                data,
                Duration::from_millis(5000),
            )?;
            Ok(n)
        }
    }

    pub fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, TransportError> {
        // timeout_ms semantics (matching hidapi):
        //   0  = non-blocking poll (check if data available, don't wait)
        //  -1  = blocking (wait indefinitely)
        //  >0  = wait up to N milliseconds
        // libusb uses 0 for "wait forever", so we remap.
        let timeout = if timeout_ms < 0 {
            Duration::from_secs(60)
        } else if timeout_ms == 0 {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(timeout_ms as u64)
        };
        match self.handle.read_interrupt(self.ep_in, buf, timeout) {
            Ok(n) => Ok(n),
            Err(rusb::Error::Timeout) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for RusbHidTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.iface);
        let _ = self.handle.attach_kernel_driver(self.iface);
    }
}

/// Parse the first Usage Page value from a HID report descriptor.
///
/// HID report descriptors use a tag-based format:
/// - `0x05, page` — Usage Page (1 byte)
/// - `0x06, lo, hi` — Usage Page (2 bytes)
fn parse_usage_page(desc: &[u8]) -> Option<u16> {
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0 {
            break; // End of descriptor
        }
        let size = (prefix & 0x03) as usize;
        let tag = prefix & 0xFC;

        // Usage Page tags: short items with tag bits 0000 01xx
        // 0x05 = 1-byte usage page, 0x06 = 2-byte usage page
        if tag == 0x04 {
            match size {
                1 if i + 1 < desc.len() => return Some(desc[i + 1] as u16),
                2 if i + 2 < desc.len() => {
                    return Some(u16::from_le_bytes([desc[i + 1], desc[i + 2]]))
                }
                _ => {}
            }
        }

        // Advance past this item: 1 byte prefix + size bytes data
        // Size value 3 means 4 bytes of data in HID descriptor encoding
        let data_len = if size == 3 { 4 } else { size };
        i += 1 + data_len;
    }
    None
}
