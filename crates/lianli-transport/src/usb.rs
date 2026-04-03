use crate::error::TransportError;
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;
use tracing::{debug, info, warn};

pub const EP_OUT: u8 = 0x01;
pub const EP_IN: u8 = 0x81;
pub const USB_TIMEOUT: Duration = Duration::from_millis(5_000);
pub const LCD_WRITE_TIMEOUT: Duration = Duration::from_millis(200);
pub const LCD_READ_TIMEOUT: Duration = Duration::from_millis(2_000);

/// Low-level USB transport wrapping a `rusb` device handle.
///
/// Auto-detects endpoint transfer types (bulk vs interrupt) from the USB
/// descriptor so the correct libusb call is used.
pub struct UsbTransport {
    handle: DeviceHandle<GlobalContext>,
    ep_out: u8,
    ep_in: u8,
    ep_in_interrupt: bool,
    ep_out_interrupt: bool,
}

impl UsbTransport {
    pub fn open(vid: u16, pid: u16) -> Result<Self, TransportError> {
        let device = rusb::devices()?
            .iter()
            .find(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
            })
            .ok_or(TransportError::DeviceNotFound { vid, pid })?;
        let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
        let handle = device.open()?;
        Ok(Self {
            handle,
            ep_out: EP_OUT,
            ep_in: EP_IN,
            ep_in_interrupt,
            ep_out_interrupt,
        })
    }

    pub fn open_device(device: Device<GlobalContext>) -> Result<Self, TransportError> {
        let (ep_in_interrupt, ep_out_interrupt) = detect_endpoint_types(&device);
        let handle = device.open()?;
        Ok(Self {
            handle,
            ep_out: EP_OUT,
            ep_in: EP_IN,
            ep_in_interrupt,
            ep_out_interrupt,
        })
    }

    pub fn detach_and_configure(&mut self, name: &str) -> Result<(), TransportError> {
        match self.handle.kernel_driver_active(0) {
            Ok(true) => {
                self.handle.detach_kernel_driver(0)?;
                debug!("Detached kernel driver from {name}");
            }
            Ok(false) => {}
            Err(rusb::Error::NotSupported) => {}
            Err(e) => return Err(e.into()),
        }

        match self.handle.set_active_configuration(1) {
            Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
            Err(rusb::Error::Io) => {
                warn!("{name} configuration I/O error, attempting USB reset");
                self.handle.reset()?;
                info!("{name} USB reset successful, retrying");
                std::thread::sleep(Duration::from_millis(500));
                // Kernel driver may re-attach after reset
                match self.handle.kernel_driver_active(0) {
                    Ok(true) => {
                        let _ = self.handle.detach_kernel_driver(0);
                        debug!("Detached kernel driver from {name} after reset");
                    }
                    _ => {}
                }
                match self.handle.set_active_configuration(1) {
                    Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) => return Err(e.into()),
        }

        match self.handle.claim_interface(0) {
            Ok(()) => {
                let _ = self.handle.set_alternate_setting(0, 0);
            }
            Err(rusb::Error::Busy) => {
                warn!("{name} interface busy, attempting USB reset");
                self.handle.reset()?;
                info!("{name} USB reset successful");
                std::thread::sleep(Duration::from_millis(500));
                // Kernel driver may re-attach after reset — detach again
                match self.handle.kernel_driver_active(0) {
                    Ok(true) => {
                        self.handle.detach_kernel_driver(0)?;
                        debug!("Detached kernel driver from {name} after reset");
                    }
                    Ok(false) => {}
                    Err(rusb::Error::NotSupported) => {}
                    Err(e) => return Err(e.into()),
                }
                self.handle.claim_interface(0)?;
                let _ = self.handle.set_alternate_setting(0, 0);
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    pub fn write(&self, data: &[u8], timeout: Duration) -> Result<usize, TransportError> {
        let n = if self.ep_out_interrupt {
            self.handle.write_interrupt(self.ep_out, data, timeout)?
        } else {
            self.handle.write_bulk(self.ep_out, data, timeout)?
        };
        if n != data.len() {
            warn!(
                "USB short write: {n}/{} bytes on EP 0x{:02x} ({})",
                data.len(), self.ep_out,
                if self.ep_out_interrupt { "interrupt" } else { "bulk" }
            );
        }
        Ok(n)
    }

    pub fn read(&self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        if self.ep_in_interrupt {
            Ok(self.handle.read_interrupt(self.ep_in, buf, timeout)?)
        } else {
            Ok(self.handle.read_bulk(self.ep_in, buf, timeout)?)
        }
    }

    /// Drain any remaining data from the read pipe.
    pub fn read_flush(&self) {
        let mut buf = [0u8; 512];
        let _ = self.read(&mut buf, Duration::from_millis(1));
    }

    pub fn release(&self) {
        let _ = self.handle.release_interface(0);
    }

    pub fn reset(&self) -> Result<(), TransportError> {
        Ok(self.handle.reset()?)
    }

    pub fn inner(&self) -> &DeviceHandle<GlobalContext> {
        &self.handle
    }

    pub fn read_serial(&self, device: &Device<GlobalContext>) -> Option<String> {
        let desc = device.device_descriptor().ok()?;
        self.handle.read_serial_number_string_ascii(&desc).ok()
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}

/// Detect whether EP_IN and EP_OUT are interrupt endpoints by reading the
/// USB descriptor. Returns `(ep_in_is_interrupt, ep_out_is_interrupt)`.
fn detect_endpoint_types(device: &Device<GlobalContext>) -> (bool, bool) {
    let config = match device.active_config_descriptor() {
        Ok(c) => c,
        Err(_) => return (false, false),
    };
    let mut in_interrupt = false;
    let mut out_interrupt = false;
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            for ep in desc.endpoint_descriptors() {
                if ep.address() == EP_IN
                    && ep.transfer_type() == rusb::TransferType::Interrupt
                {
                    in_interrupt = true;
                }
                if ep.address() == EP_OUT
                    && ep.transfer_type() == rusb::TransferType::Interrupt
                {
                    out_interrupt = true;
                }
            }
        }
    }
    debug!(
        "Endpoint types: IN=0x{:02x} {}, OUT=0x{:02x} {}",
        EP_IN,
        if in_interrupt { "interrupt" } else { "bulk" },
        EP_OUT,
        if out_interrupt { "interrupt" } else { "bulk" },
    );
    (in_interrupt, out_interrupt)
}

/// Find all USB devices matching a VID/PID, sorted by bus/address.
pub fn find_usb_devices(vid: u16, pid: u16) -> Result<Vec<Device<GlobalContext>>, TransportError> {
    let devices = rusb::devices()?;
    let mut list = Vec::new();
    for device in devices.iter() {
        if let Ok(desc) = device.device_descriptor() {
            if desc.vendor_id() == vid && desc.product_id() == pid {
                list.push(device);
            }
        }
    }
    list.sort_by_key(|dev| (dev.bus_number(), dev.address()));
    Ok(list)
}
