//! USB bulk / WinUSB-style transport.
//!
//! Wraps a `rusb::DeviceHandle` for devices that speak bulk or interrupt
//! transfers on fixed endpoints (EP 0x01 out, EP 0x81 in). Used by every
//! non-HID USB device: wireless dongles, WinUSB LCDs (HydroShift II / Lancool
//! 207 / Universal Screen), WinUSB LED controllers, and TURZX desktop-mode
//! displays.

use crate::error::TransportError;
use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;
use tracing::{debug, warn};

/// Default OUT endpoint address (vendor-defined but consistent across the
/// Lian Li USB-bulk fleet).
pub const EP_OUT: u8 = 0x01;
/// Default IN endpoint address.
pub const EP_IN: u8 = 0x81;
/// Default timeout for ordinary control transfers.
pub const USB_TIMEOUT: Duration = Duration::from_millis(5_000);
/// Per-frame write timeout for LCD streaming (tight to keep frame pacing tight).
pub const LCD_WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// Per-frame read timeout for LCD status polling.
pub const LCD_READ_TIMEOUT: Duration = Duration::from_millis(2_000);

/// USB bulk transport wrapping a `rusb::DeviceHandle`.
///
/// Auto-detects endpoint transfer types (bulk vs interrupt) from the USB
/// descriptor so the correct libusb call is used.

/// Set once the daemon starts shutting down. Long retry loops in this module
/// poll it so a worker thread never sits inside a multi-second USB retry while
/// `shutdown()` is blocked joining it — that stall is what forced the process
/// to exit with a transfer still in flight, which hangs the device MCU.
pub static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True once shutdown has begun; long loops must bail out promptly.
pub fn shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::Relaxed)
}

pub struct RusbBulk {
    handle: DeviceHandle<GlobalContext>,
    ep_out: u8,
    ep_in: u8,
    ep_in_interrupt: bool,
    ep_out_interrupt: bool,
    /// All interfaces we hold for the lifetime of this transport.
    /// Held continuously so the kernel can't re-bind and reject our writes.
    claimed: Vec<u8>,
}

impl RusbBulk {
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
            claimed: Vec::new(),
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
            claimed: Vec::new(),
        })
    }

    /// Detach any kernel driver, set the active configuration, and claim
    /// interface 0 (plus any other vendor interfaces). Recovers from a busy
    /// state by retrying with short delays rather than USB reset (which can
    /// destabilise other devices on the same hub).
    pub fn detach_and_configure(&mut self, name: &str) -> Result<(), TransportError> {
        self.detach_and_configure_with_cancel(name, || false)
    }

    pub fn detach_and_configure_with_cancel(
        &mut self,
        name: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), TransportError> {
        if shutting_down() || cancelled() {
            return Err(rusb::Error::Interrupted.into());
        }
        match self.handle.kernel_driver_active(0) {
            Ok(true) => {
                self.handle.detach_kernel_driver(0)?;
                debug!("Detached kernel driver from {name}");
            }
            Ok(false) => {}
            Err(rusb::Error::NotSupported) => {}
            Err(e) => return Err(e.into()),
        }

        let need_reconfig = self
            .handle
            .device()
            .active_config_descriptor()
            .ok()
            .map(|c| c.number() != 1)
            .unwrap_or(true);
        if need_reconfig {
            match self.handle.set_active_configuration(1) {
                Ok(()) | Err(rusb::Error::Busy) | Err(rusb::Error::NotFound) => {}
                Err(e) => {
                    debug!("{name} set_active_configuration: {e}, continuing");
                }
            }
        }

        match self.handle.claim_interface(0) {
            Ok(()) => {
                let _ = self.handle.set_alternate_setting(0, 0);
                self.claimed.push(0);
            }
            Err(rusb::Error::Busy) => {
                // A busy interface is expected once shutdown starts, since
                // handles are still held while their owners are being
                // joined. Do not warn or sit through the retry loop.
                if shutting_down() || cancelled() {
                    debug!("{name} interface 0 busy while shutting down, not retrying");
                    return Err(rusb::Error::Busy.into());
                }
                warn!("{name} interface 0 busy, retrying...");
                let mut claimed = false;
                for attempt in 1..=20u32 {
                    if shutting_down() || cancelled() {
                        debug!("{name}: aborting interface claim, shutting down");
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                    if shutting_down() || cancelled() {
                        return Err(rusb::Error::Interrupted.into());
                    }
                    if let Ok(true) = self.handle.kernel_driver_active(0) {
                        let _ = self.handle.detach_kernel_driver(0);
                    }
                    match self.handle.claim_interface(0) {
                        Ok(()) => {
                            claimed = true;
                            break;
                        }
                        Err(rusb::Error::Busy) => {
                            debug!("{name} interface 0 still busy (attempt {attempt}/20)");
                            continue;
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                if !claimed {
                    return Err(rusb::Error::Busy.into());
                }
                let _ = self.handle.set_alternate_setting(0, 0);
                self.claimed.push(0);
            }
            Err(e) => return Err(e.into()),
        }

        if let Ok(config) = self.handle.device().active_config_descriptor() {
            for iface in config.interfaces() {
                let num = iface.number();
                if num == 0 || self.claimed.contains(&num) {
                    continue;
                }
                if let Ok(true) = self.handle.kernel_driver_active(num) {
                    let _ = self.handle.detach_kernel_driver(num);
                }
                match self.handle.claim_interface(num) {
                    Ok(()) => {
                        self.claimed.push(num);
                        debug!("{name}: claimed extra interface {num}");
                    }
                    Err(e) => warn!("{name}: claim extra interface {num} failed: {e}"),
                }
            }
        }

        Ok(())
    }

    pub fn write(&self, data: &[u8], timeout: Duration) -> Result<usize, TransportError> {
        // FIX: refuse to start new transfers once shutdown begins. The main
        // loop was blocking inside device_poll()'s USB reads (2s timeout each)
        // and never reached the Shutdown event, so shutdown() never ran and the
        // process was forced down mid-transfer — which hangs the device MCU.
        // Refusing *new* transfers is safe: in-flight ones still drain within
        // their own timeout, so handlers unwind quickly and cleanly.
        if shutting_down() {
            return Err(TransportError::Usb(rusb::Error::Interrupted));
        }
        let n = if self.ep_out_interrupt {
            self.handle.write_interrupt(self.ep_out, data, timeout)?
        } else {
            self.handle.write_bulk(self.ep_out, data, timeout)?
        };
        if n != data.len() {
            warn!(
                "USB short write: {n}/{} bytes on EP 0x{:02x} ({})",
                data.len(),
                self.ep_out,
                if self.ep_out_interrupt {
                    "interrupt"
                } else {
                    "bulk"
                }
            );
        }
        Ok(n)
    }

    /// Write all data, handling short writes by continuing from the offset
    /// where the previous transfer left off. Each sub-transfer uses the same
    /// timeout, so the total worst-case is `timeout * number_of_chunks`.
    pub fn write_full(&self, data: &[u8], timeout: Duration) -> Result<(), TransportError> {
        // FIX: refuse to start new transfers once shutdown begins. The main
        // loop was blocking inside device_poll()'s USB reads (2s timeout each)
        // and never reached the Shutdown event, so shutdown() never ran and the
        // process was forced down mid-transfer — which hangs the device MCU.
        // Refusing *new* transfers is safe: in-flight ones still drain within
        // their own timeout, so handlers unwind quickly and cleanly.
        if shutting_down() {
            return Err(TransportError::Usb(rusb::Error::Interrupted));
        }
        let mut offset = 0usize;
        while offset < data.len() {
            let n = if self.ep_out_interrupt {
                self.handle
                    .write_interrupt(self.ep_out, &data[offset..], timeout)?
            } else {
                self.handle
                    .write_bulk(self.ep_out, &data[offset..], timeout)?
            };
            if n == 0 {
                return Err(TransportError::Write(format!(
                    "zero-length USB write at offset {offset}/{}",
                    data.len()
                )));
            }
            offset += n;
        }
        Ok(())
    }

    pub fn read(&self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError> {
        // FIX: refuse to start new transfers once shutdown begins. The main
        // loop was blocking inside device_poll()'s USB reads (2s timeout each)
        // and never reached the Shutdown event, so shutdown() never ran and the
        // process was forced down mid-transfer — which hangs the device MCU.
        // Refusing *new* transfers is safe: in-flight ones still drain within
        // their own timeout, so handlers unwind quickly and cleanly.
        if shutting_down() {
            return Err(TransportError::Usb(rusb::Error::Interrupted));
        }
        if self.ep_in_interrupt {
            Ok(self.handle.read_interrupt(self.ep_in, buf, timeout)?)
        } else {
            Ok(self.handle.read_bulk(self.ep_in, buf, timeout)?)
        }
    }

    pub fn control_in(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buf: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, TransportError> {
        if shutting_down() {
            return Err(TransportError::Usb(rusb::Error::Interrupted));
        }
        Ok(self
            .handle
            .read_control(request_type, request, value, index, buf, timeout)?)
    }

    pub fn control_out(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, TransportError> {
        if shutting_down() {
            return Err(TransportError::Usb(rusb::Error::Interrupted));
        }
        Ok(self
            .handle
            .write_control(request_type, request, value, index, data, timeout)?)
    }

    /// Drain any remaining data from the read pipe.
    pub fn read_flush(&self) {
        let mut buf = [0u8; 512];
        loop {
            match self.read(&mut buf, Duration::from_millis(5)) {
                Ok(n) if n > 0 => continue,
                _ => break,
            }
        }
    }

    /// Read chunks until silence (mirrors WinUsb `ReadAll`). Returns total bytes.
    pub fn read_silence(
        &self,
        buf: &mut [u8],
        first_timeout: Duration,
        chunk_timeout: Duration,
    ) -> usize {
        let mut chunk = [0u8; 64];
        let mut total = 0usize;
        let mut timeout = first_timeout;
        loop {
            match self.read(&mut chunk, timeout) {
                Ok(n) if n > 0 => {
                    timeout = chunk_timeout;
                    let n = n.min(buf.len() - total);
                    buf[total..total + n].copy_from_slice(&chunk[..n]);
                    total += n;
                    if total == buf.len() {
                        break;
                    }
                }
                _ => break,
            }
        }
        total
    }

    /// Silence ends a response successfully; other USB errors discard partial data.
    pub fn read_silence_checked(
        &self,
        buf: &mut [u8],
        first_timeout: Duration,
        chunk_timeout: Duration,
    ) -> Result<usize, TransportError> {
        read_chunks_until_silence(buf, first_timeout, chunk_timeout, |chunk, timeout| {
            self.read(chunk, timeout)
        })
    }

    pub fn release(&self) {
        for &iface in self.claimed.iter().rev() {
            let _ = self.handle.release_interface(iface);
        }
    }

    pub fn reset(&self) -> Result<(), TransportError> {
        Ok(self.handle.reset()?)
    }

    pub fn clear_halt(&self, endpoint: u8) -> Result<(), TransportError> {
        Ok(self.handle.clear_halt(endpoint)?)
    }

    pub fn inner(&self) -> &DeviceHandle<GlobalContext> {
        &self.handle
    }

    pub fn read_serial(&self, device: &Device<GlobalContext>) -> Option<String> {
        let desc = device.device_descriptor().ok()?;
        self.handle.read_serial_number_string_ascii(&desc).ok()
    }
}

impl Drop for RusbBulk {
    fn drop(&mut self) {
        for &iface in self.claimed.iter().rev() {
            let _ = self.handle.release_interface(iface);
            let _ = self.handle.attach_kernel_driver(iface);
        }
    }
}

fn read_chunks_until_silence(
    buf: &mut [u8],
    first_timeout: Duration,
    chunk_timeout: Duration,
    mut read: impl FnMut(&mut [u8], Duration) -> Result<usize, TransportError>,
) -> Result<usize, TransportError> {
    let mut total = 0;
    let mut timeout = first_timeout;
    let mut chunk = [0u8; 64];
    while total < buf.len() {
        match read(&mut chunk, timeout) {
            Ok(0)
            | Err(TransportError::Usb(rusb::Error::Timeout))
            | Err(TransportError::Timeout) => break,
            Ok(n) => {
                let n = n.min(chunk.len()).min(buf.len() - total);
                buf[total..total + n].copy_from_slice(&chunk[..n]);
                total += n;
                timeout = chunk_timeout;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(total)
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
                if ep.address() == EP_IN && ep.transfer_type() == rusb::TransferType::Interrupt {
                    in_interrupt = true;
                }
                if ep.address() == EP_OUT && ep.transfer_type() == rusb::TransferType::Interrupt {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_read_returns_empty_response_on_initial_timeout() {
        let result = read_chunks_until_silence(&mut [0; 512], USB_TIMEOUT, USB_TIMEOUT, |_, _| {
            Err(rusb::Error::Timeout.into())
        });
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn checked_read_keeps_partial_response_on_silence() {
        let first = Duration::from_millis(100);
        let next = Duration::from_millis(10);
        let mut calls = 0;
        let mut buf = [0; 512];
        let len = read_chunks_until_silence(&mut buf, first, next, |chunk, timeout| {
            calls += 1;
            if calls == 1 {
                assert_eq!(timeout, first);
                chunk[..3].copy_from_slice(&[0x10, 0, 0x80]);
                Ok(3)
            } else {
                assert_eq!(timeout, next);
                Err(rusb::Error::Timeout.into())
            }
        })
        .unwrap();
        assert_eq!(len, 3);
        assert_eq!(&buf[..len], &[0x10, 0, 0x80]);
    }

    #[test]
    fn checked_read_reports_disconnect_after_partial_response() {
        let mut calls = 0;
        let result =
            read_chunks_until_silence(&mut [0; 512], USB_TIMEOUT, USB_TIMEOUT, |chunk, _| {
                calls += 1;
                if calls == 1 {
                    chunk.fill(0x10);
                    Ok(64)
                } else {
                    Err(rusb::Error::NoDevice.into())
                }
            });
        assert!(matches!(
            result,
            Err(TransportError::Usb(rusb::Error::NoDevice))
        ));
    }

    #[test]
    fn checked_read_stops_at_capacity_and_empty_buffers_do_not_read() {
        let mut calls = 0;
        let mut buf = [0; 64];
        assert_eq!(
            read_chunks_until_silence(&mut buf, USB_TIMEOUT, USB_TIMEOUT, |chunk, _| {
                calls += 1;
                chunk.fill(9);
                Ok(64)
            })
            .unwrap(),
            64
        );
        assert_eq!(calls, 1);
        assert_eq!(buf, [9; 64]);
        assert_eq!(
            read_chunks_until_silence(&mut [], USB_TIMEOUT, USB_TIMEOUT, |_, _| panic!(
                "empty buffer"
            ))
            .unwrap(),
            0
        );
    }
}
