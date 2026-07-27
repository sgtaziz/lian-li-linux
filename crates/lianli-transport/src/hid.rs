//! HID-over-USB transport implemented directly on `rusb` (libusb).
//!
//! This replaces both the old `hidapi` backend and the `HidBackend` wrapper.
//! All HID I/O now goes through a single type: [`RusbHid`].
//!
//! The reopen-on-failure logic that used to live in `HidBackend` is folded
//! directly into `RusbHid`. Callers that want self-healing construct with
//! [`RusbHid::with_reopener`]; callers that don't (e.g. short-lived probes)
//! just use [`RusbHid::open_by_usage`].

use crate::error::TransportError;
use rusb::{Device, DeviceHandle, GlobalContext};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Closure that produces a fresh [`RusbHid`] after a stale-handle event
/// (USB suspend/resume, hub reset, transient unplug).
///
/// Wired in at construction so the transport can self-heal without each
/// controller having to plumb its own retry logic.
pub type RusbHidReopener = Arc<dyn Fn() -> anyhow::Result<RusbHid> + Send + Sync>;

/// HID transport speaking HID-over-libusb (control transfers for feature /
/// input / output reports, plus interrupt transfers for fast IN/OUT streams).
///
/// Holds the underlying `rusb::DeviceHandle` and the claimed interface number.
/// On `Drop`, all claimed interfaces are released and the kernel driver is
/// reattached.
pub struct RusbHid {
    handle: DeviceHandle<GlobalContext>,
    iface: u8,
    /// All HID interfaces we hold for the lifetime of this transport, so the
    /// kernel can't re-bind hidraw and reject our writes.
    claimed: Vec<u8>,
    ep_in: u8,
    ep_out: Option<u8>,
    /// Optional self-healing reopener. When set, an I/O error triggers a
    /// fresh open + retry.
    reopener: Option<RusbHidReopener>,
}

impl RusbHid {
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
        Self::open_by_usage_impl(device, usage_page, None)
    }

    /// Like [`open_by_usage`](Self::open_by_usage) but with a reopener closure
    /// for self-healing on I/O failure.
    pub fn open_by_usage_with_reopener(
        device: Device<GlobalContext>,
        usage_page: Option<u16>,
        reopener: RusbHidReopener,
    ) -> Result<Self, TransportError> {
        Self::open_by_usage_impl(device, usage_page, Some(reopener))
    }

    fn open_by_usage_impl(
        device: Device<GlobalContext>,
        usage_page: Option<u16>,
        reopener: Option<RusbHidReopener>,
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

        let mut claimed: Vec<u8> = Vec::new();
        for &iface_num in &hid_ifaces {
            detach_kernel_driver_with_retry(&handle, iface_num);
            match handle.claim_interface(iface_num) {
                Ok(()) => claimed.push(iface_num),
                Err(e) => warn!("RusbHid: claim interface {iface_num} failed: {e}"),
            }
        }

        if claimed.is_empty() {
            return Err(TransportError::Other(
                "RusbHid: could not claim any HID interface".into(),
            ));
        }

        let target_iface = if let Some(required_page) = usage_page {
            let mut matched = None;
            for &iface_num in &claimed {
                let mut buf = [0u8; 256];
                let result = handle.read_control(
                    0x81,
                    0x06,
                    (0x22u16) << 8,
                    iface_num as u16,
                    &mut buf,
                    Duration::from_millis(250),
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
                    "RusbHid: no interface matched usage page {required_page:#06x}, using first claimed"
                );
                claimed[0]
            })
        } else {
            claimed[0]
        };

        claimed.retain(|&iface_num| {
            if iface_num == target_iface {
                true
            } else {
                let _ = handle.release_interface(iface_num);
                let _ = handle.attach_kernel_driver(iface_num);
                false
            }
        });

        let mut ins: Vec<u8> = Vec::new();
        let mut outs: Vec<u8> = Vec::new();
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
                        rusb::Direction::In => ins.push(ep.address()),
                        rusb::Direction::Out => outs.push(ep.address()),
                    }
                }
            }
        }
        if ins.len() > 1 {
            warn!(
                "RusbHid: interface {target_iface} has multiple interrupt IN endpoints {ins:02x?}, using first"
            );
        }
        if outs.len() > 1 {
            warn!(
                "RusbHid: interface {target_iface} has multiple interrupt OUT endpoints {outs:02x?}, using first"
            );
        }
        let ep_in = ins.first().copied();
        let ep_out = outs.first().copied();

        let ep_in = ep_in.ok_or_else(|| {
            TransportError::Other("RusbHid: no interrupt IN endpoint found".into())
        })?;

        if ep_out.is_some() {
            debug!(
                "RusbHid: interface={target_iface} ep_in=0x{ep_in:02x} ep_out=0x{:02x}",
                ep_out.unwrap()
            );
        } else {
            debug!("RusbHid: interface={target_iface} ep_in=0x{ep_in:02x} (using SET_REPORT for writes)");
        }

        Ok(Self {
            handle,
            iface: target_iface,
            claimed,
            ep_in,
            ep_out,
            reopener,
        })
    }

    /// Attach a reopener after construction. Less common than
    /// [`open_by_usage_with_reopener`](Self::open_by_usage_with_reopener) —
    /// used when the reopener needs the same `RusbHid` it's constructing.
    pub fn set_reopener(&mut self, reopener: RusbHidReopener) {
        self.reopener = Some(reopener);
    }

    /// Perform a USB port reset on the device (USBDEVFS_RESET ioctl).
    /// Used during device binding/unbinding to force kernel re-enumeration.
    pub fn reset_usb_device(device: &Device<GlobalContext>) -> Result<(), TransportError> {
        let handle = device.open()?;

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

        for &iface in &hid_ifaces {
            if let Ok(true) = handle.kernel_driver_active(iface) {
                let _ = handle.detach_kernel_driver(iface);
            }
        }

        handle
            .reset()
            .map_err(|e| TransportError::Other(format!("USB device reset failed: {e}")))?;

        for &iface in &hid_ifaces {
            let _ = handle.attach_kernel_driver(iface);
        }

        Ok(())
    }

    // ----- reopen machinery ---------------------------------------------------

    fn try_reopen(&mut self) -> Result<(), TransportError> {
        let reopener = self
            .reopener
            .clone()
            .ok_or_else(|| TransportError::Other("no reopener configured".into()))?;
        let replacement = reopener().map_err(|e| TransportError::Other(format!("reopen: {e}")))?;
        *self = replacement;
        Ok(())
    }

    /// Run `op` against the inner handle. If it fails and a reopener is
    /// configured, reopen once and retry.
    fn with_reopen<T>(
        &mut self,
        mut op: impl FnMut(&Self) -> Result<T, TransportError>,
        label: &str,
    ) -> Result<T, TransportError> {
        match op(self) {
            Ok(v) => Ok(v),
            Err(e) if self.reopener.is_some() => {
                warn!("RusbHid {label} failed ({e}); attempting reopen");
                self.try_reopen()?;
                info!("RusbHid handle reopened, retrying {label}");
                op(self)
            }
            Err(e) => Err(e),
        }
    }

    // ----- HID report API -----------------------------------------------------

    pub fn send_feature_report(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        self.with_reopen(
            |s| {
                let report_id = data.first().copied().unwrap_or(0) as u16;
                let w_value = (0x03u16 << 8) | report_id;
                s.handle
                    .write_control(
                        0x21,
                        0x09,
                        w_value,
                        s.iface as u16,
                        data,
                        Duration::from_millis(5000),
                    )
                    .map_err(TransportError::from)
            },
            "send_feature_report",
        )
    }

    pub fn get_feature_report(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.with_reopen(
            |s| {
                let report_id = buf.first().copied().unwrap_or(0) as u16;
                let w_value = (0x03u16 << 8) | report_id;
                s.handle
                    .read_control(
                        0xA1,
                        0x01,
                        w_value,
                        s.iface as u16,
                        buf,
                        Duration::from_millis(5000),
                    )
                    .map_err(TransportError::from)
            },
            "get_feature_report",
        )
    }

    pub fn get_input_report(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.with_reopen(
            |s| {
                let report_id = buf.first().copied().unwrap_or(0) as u16;
                let w_value = (0x01u16 << 8) | report_id;
                s.handle
                    .read_control(
                        0xA1,
                        0x01,
                        w_value,
                        s.iface as u16,
                        buf,
                        Duration::from_millis(5000),
                    )
                    .map_err(TransportError::from)
            },
            "get_input_report",
        )
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        self.with_reopen(
            |s| {
                if let Some(ep_out) = s.ep_out {
                    s.handle
                        .write_interrupt(ep_out, data, Duration::from_millis(5000))
                        .map_err(TransportError::from)
                } else {
                    // SET_REPORT control transfer: report type = Output (0x02),
                    // report ID = data[0]
                    let report_id = data.first().copied().unwrap_or(0) as u16;
                    let report_type: u16 = 0x02;
                    let w_value = (report_type << 8) | report_id;
                    s.handle
                        .write_control(
                            0x21,
                            0x09,
                            w_value,
                            s.iface as u16,
                            data,
                            Duration::from_millis(5000),
                        )
                        .map_err(TransportError::from)
                }
            },
            "write",
        )
    }

    pub fn read_timeout(
        &mut self,
        buf: &mut [u8],
        timeout_ms: i32,
    ) -> Result<usize, TransportError> {
        self.with_reopen(
            |s| {
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
                match s.handle.read_interrupt(s.ep_in, buf, timeout) {
                    Ok(n) => Ok(n),
                    Err(rusb::Error::Timeout) => Ok(0),
                    Err(e) => Err(TransportError::from(e)),
                }
            },
            "read_timeout",
        )
    }

    /// Drain any stale data from the device read buffer.
    pub fn read_flush(&mut self) {
        let mut buf = [0u8; 64];
        loop {
            // Direct read (no reopen) since this is best-effort cleanup.
            let timeout = Duration::from_millis(5);
            match self.handle.read_interrupt(self.ep_in, &mut buf, timeout) {
                Ok(n) if n > 0 => continue,
                _ => break,
            }
        }
    }
}

impl Drop for RusbHid {
    fn drop(&mut self) {
        for &iface in self.claimed.iter().rev() {
            let _ = self.handle.release_interface(iface);
            let _ = self.handle.attach_kernel_driver(iface);
        }
    }
}

fn detach_kernel_driver_with_retry(handle: &DeviceHandle<GlobalContext>, iface: u8) {
    for attempt in 0..2 {
        let active = handle.kernel_driver_active(iface);
        let needs_detach = matches!(active, Ok(true) | Err(_));
        if !needs_detach {
            return;
        }
        match handle.detach_kernel_driver(iface) {
            Ok(()) => {
                debug!("RusbHid: detached kernel driver from interface {iface}");
                return;
            }
            Err(rusb::Error::NotFound) => return,
            Err(e) if attempt == 0 => {
                std::thread::sleep(Duration::from_millis(50));
                debug!("RusbHid: detach interface {iface} retry after error: {e}");
            }
            Err(e) => {
                warn!("RusbHid: detach interface {iface} failed: {e}");
                return;
            }
        }
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
