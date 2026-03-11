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
    pub fn open(device: Device<GlobalContext>, iface: u8) -> Result<Self, TransportError> {
        let handle = device.open()?;

        match handle.kernel_driver_active(iface) {
            Ok(true) => {
                handle.detach_kernel_driver(iface)?;
                debug!("RusbHid: detached kernel driver from interface {iface}");
            }
            Ok(false) => {}
            Err(rusb::Error::NotSupported) => {}
            Err(e) => return Err(e.into()),
        }

        handle.claim_interface(iface)?;

        let config = device.active_config_descriptor()?;
        let mut ep_in: Option<u8> = None;
        let mut ep_out: Option<u8> = None;

        for iface_group in config.interfaces() {
            for desc in iface_group.descriptors() {
                if desc.interface_number() != iface {
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
            debug!("RusbHid: interface={iface} ep_in=0x{ep_in:02x} ep_out=0x{:02x}", ep_out.unwrap());
        } else {
            debug!("RusbHid: interface={iface} ep_in=0x{ep_in:02x} (using SET_REPORT for writes)");
        }

        Ok(Self {
            handle,
            iface,
            ep_in,
            ep_out,
        })
    }

    pub fn find_hid_interface(device: &Device<GlobalContext>) -> Option<u8> {
        let config = device.active_config_descriptor().ok()?;
        for iface in config.interfaces() {
            for desc in iface.descriptors() {
                if desc.class_code() == 0x03 {
                    return Some(desc.interface_number());
                }
            }
        }
        None
    }

    /// Send a HID Feature report via SET_REPORT control transfer (report type 0x03).
    /// Does not require an interrupt OUT endpoint — works with control-transfer-only devices.
    pub fn send_feature(&self, data: &[u8]) -> Result<usize, TransportError> {
        let report_id = data.first().copied().unwrap_or(0) as u16;
        let w_value = (0x03u16 << 8) | report_id; // 0x03 = Feature report type
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

    /// Read a HID Feature report via GET_REPORT control transfer (report type 0x03).
    pub fn get_feature(&self, report_id: u8, buf: &mut [u8]) -> Result<usize, TransportError> {
        let w_value = (0x03u16 << 8) | report_id as u16; // 0x03 = Feature report type
        let n = self.handle.read_control(
            0xA1, // Device-to-host, Class, Interface
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
        let timeout = if timeout_ms < 0 {
            Duration::from_secs(60)
        } else {
            Duration::from_millis(timeout_ms as u64)
        };
        // Use a larger internal buffer to avoid Overflow when the device
        // sends more than the caller expects, then copy what fits.
        let mut tmp = [0u8; 4096];
        match self.handle.read_interrupt(self.ep_in, &mut tmp, timeout) {
            Ok(n) => {
                let copy_len = n.min(buf.len());
                buf[..copy_len].copy_from_slice(&tmp[..copy_len]);
                Ok(copy_len)
            }
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
