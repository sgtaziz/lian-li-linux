use super::enumerate::find_usb_device;
use super::DetectedDevice;
use anyhow::Result;
use lianli_shared::device_id::DeviceFamily;
use lianli_transport::{RusbBulk, RusbHid, RusbHidReopener};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// Build a reopener that re-acquires the same HID device via the rusb backend.
/// Matches by USB topology (bus + port_numbers) to disambiguate multiple
/// devices sharing the same VID:PID (e.g. daisy-chained TL LCD fans).
fn make_rusb_reopener(
    vid: u16,
    pid: u16,
    bus: u8,
    port_numbers: Vec<u8>,
    usage_page: Option<u16>,
) -> RusbHidReopener {
    Arc::new(move || {
        let usb_dev = rusb::devices()
            .map_err(|e| anyhow::anyhow!("rusb devices: {e}"))?
            .iter()
            .find(|d| {
                d.bus_number() == bus
                    && d.port_numbers().ok().as_deref() == Some(&port_numbers[..])
                    && d.device_descriptor()
                        .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                        .unwrap_or(false)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "USB device {vid:04x}:{pid:04x} at {bus}-{:?} not enumerable on reopen",
                    port_numbers
                )
            })?;
        let transport = RusbHid::open_by_usage_with_reopener(
            usb_dev,
            usage_page,
            make_rusb_reopener(vid, pid, bus, port_numbers.clone(), usage_page),
        )
        .map_err(|e| anyhow::anyhow!("rusb hid open: {e}"))?;
        Ok(transport)
    })
}

/// Try opening a device, retrying on failure. First two retries are plain
/// reopens, only the last retry does a USB port reset.
fn try_open_with_retry<T>(
    usb_device: Option<&Device<GlobalContext>>,
    label: &str,
    mut open_fn: impl FnMut() -> Result<T>,
) -> Result<T> {
    const MAX_RETRIES: u32 = 3;
    const RESET_AT: u32 = 2;
    for attempt in 0..=MAX_RETRIES {
        match open_fn() {
            Ok(t) => return Ok(t),
            Err(e) if attempt < MAX_RETRIES => {
                if attempt == RESET_AT {
                    if let Some(usb_dev) = usb_device {
                        warn!(
                            "{label}: open attempt {} failed: {e}, resetting USB device",
                            attempt + 1
                        );
                        let _ = RusbHid::reset_usb_device(usb_dev);
                        std::thread::sleep(Duration::from_secs(3));
                    } else {
                        return Err(e.context(format!(
                            "{label}: failed and no USB device available for reset"
                        )));
                    }
                } else {
                    warn!(
                        "{label}: open attempt {} failed: {e}, retrying",
                        attempt + 1
                    );
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
            Err(e) => {
                return Err(e.context(format!(
                    "{label}: failed after {} attempts",
                    MAX_RETRIES + 1
                )));
            }
        }
    }
    unreachable!()
}

fn open_with_retry<T>(
    usb_device: &Device<GlobalContext>,
    open_fn: impl FnMut() -> Result<T>,
) -> Result<T> {
    try_open_with_retry(Some(usb_device), "rusb open", open_fn)
}

/// Open a detected HID-capable device as an LCD controller via the rusb HID
/// backend. Returns `None` for families that don't expose an LCD.
pub fn open_hid_lcd_device(
    det: &DetectedDevice,
) -> Option<Result<Box<dyn crate::traits::LcdDevice>>> {
    let pid = det.pid;
    match det.family {
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => {
            let vid = det.vid;
            let bus = det.bus;
            let port_numbers = det.device.port_numbers().unwrap_or_default();
            let usage_page = det.hid_usage_page;
            let usb_device = det.device.clone();
            Some(open_with_retry(&det.device, || {
                let transport = RusbHid::open_by_usage_with_reopener(
                    usb_device.clone(),
                    usage_page,
                    make_rusb_reopener(vid, pid, bus, port_numbers.clone(), usage_page),
                )?;
                let mut backend = transport;
                backend.read_flush();
                let backend = Arc::new(Mutex::new(backend));
                crate::hydroshift_lcd::HydroShiftLcdController::new(backend, pid)
                    .map(|d| Box::new(d) as Box<dyn crate::traits::LcdDevice>)
            }))
        }
        DeviceFamily::TlLcd => {
            let vid = det.vid;
            let bus = det.bus;
            let port_numbers = det.device.port_numbers().unwrap_or_default();
            let usage_page = det.hid_usage_page;
            let usb_device = det.device.clone();
            Some(open_with_retry(&det.device, || {
                let transport = RusbHid::open_by_usage_with_reopener(
                    usb_device.clone(),
                    usage_page,
                    make_rusb_reopener(vid, pid, bus, port_numbers.clone(), usage_page),
                )?;
                let backend = Arc::new(Mutex::new(transport));
                let mut tl = crate::tl_lcd::TlLcdDevice::new(backend);
                crate::traits::LcdDevice::initialize(&mut tl)?;
                Ok(Box::new(tl) as Box<dyn crate::traits::LcdDevice>)
            }))
        }
        _ => None,
    }
}

fn usb_topology_string(bus: u8, port_numbers: &[u8]) -> String {
    format!(
        "{}-{}",
        bus,
        port_numbers
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(".")
    )
}

/// Resolve the `/dev/hidrawN` path for a USB device by topology.
///
/// Walks `/sys/class/hidraw` looking for the canonicalized device symlink
/// whose target contains the matching `{bus}-{port.port.port}` string.
pub fn hidraw_path_for_usb_topology(bus: u8, port_numbers: &[u8]) -> Option<std::ffi::CString> {
    if port_numbers.is_empty() {
        return None;
    }
    let topology = usb_topology_string(bus, port_numbers);
    let needles = [format!("/{topology}/"), format!("/{topology}:")];
    let class_dir = std::path::Path::new("/sys/class/hidraw");
    for entry in std::fs::read_dir(class_dir).ok()?.flatten() {
        let Ok(resolved) = std::fs::canonicalize(entry.path()) else {
            continue;
        };
        let resolved_str = resolved.to_string_lossy();
        if needles.iter().any(|n| resolved_str.contains(n.as_str())) {
            let name = entry.file_name();
            let name = name.to_str()?;
            return std::ffi::CString::new(format!("/dev/{name}")).ok();
        }
    }
    None
}

/// Open a HID LCD device by USB topology (bus + port path).
///
/// Required for devices like TL LCD where multiple physical units share
/// VID:PID — the topology pinpoints which one to open.
pub fn open_hid_lcd_by_topology(
    vid: u16,
    pid: u16,
    family: DeviceFamily,
    bus: u8,
    port_numbers: &[u8],
) -> Result<Box<dyn crate::traits::LcdDevice>> {
    let device = rusb::devices()?
        .iter()
        .find(|d| {
            d.bus_number() == bus
                && d.port_numbers().ok().as_deref() == Some(port_numbers)
                && d.device_descriptor()
                    .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
                    .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no USB device matching topology {} for {vid:04x}:{pid:04x}",
                usb_topology_string(bus, port_numbers)
            )
        })?;
    let det = DetectedDevice {
        device: device.clone(),
        family,
        vid,
        pid,
        name: "",
        bus,
        address: device.address(),
        serial: None,
        hid_usage_page: None,
    };
    match open_hid_lcd_device(&det) {
        Some(Ok(ctrl)) => Ok(ctrl),
        Some(Err(e)) => Err(e.context("HID LCD open by topology")),
        None => Err(anyhow::anyhow!("family does not support LCD")),
    }
}

/// Open a HID LCD device by VID/PID with retry logic.
///
/// Performs USB reset + re-enumeration between attempts so a missing HID
/// interface (kernel didn't bind `usbhid` at boot) gets a second chance.
pub fn open_hid_lcd_by_vid_pid(
    vid: u16,
    pid: u16,
    family: DeviceFamily,
) -> Result<Box<dyn crate::traits::LcdDevice>> {
    let usb_device = find_usb_device(vid, pid);

    for attempt in 0..=3u32 {
        if let Some(device) = usb_device.as_ref() {
            let det = DetectedDevice {
                device: device.clone(),
                family,
                vid,
                pid,
                name: "",
                bus: device.bus_number(),
                address: device.address(),
                serial: None,
                hid_usage_page: None,
            };
            match open_hid_lcd_device(&det) {
                Some(Ok(ctrl)) => return Ok(ctrl),
                Some(Err(e)) if attempt < 3 => {
                    warn!(
                        "HID LCD open attempt {} failed ({vid:04x}:{pid:04x}): {e}, resetting USB",
                        attempt + 1
                    );
                }
                Some(Err(e)) => {
                    return Err(e.context("HID LCD open failed after 4 attempts"));
                }
                None => return Err(anyhow::anyhow!("family does not support LCD")),
            }
        } else if attempt < 3 {
            warn!(
                "No USB device {vid:04x}:{pid:04x} found (attempt {}), retrying",
                attempt + 1
            );
        } else {
            return Err(anyhow::anyhow!(
                "no USB device found for {vid:04x}:{pid:04x} after 4 attempts"
            ));
        }

        if let Some(ref usb_dev) = usb_device {
            let _ = RusbHid::reset_usb_device(usb_dev);
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    unreachable!()
}

/// Open a shared HID backend via rusb with retry logic.
/// Returns an `Arc<Mutex<RusbHid>>` that can be shared between multiple
/// controllers.
pub fn open_hid_backend(det: &DetectedDevice) -> Result<Arc<Mutex<RusbHid>>> {
    open_hid_with_reopener(
        det.device.clone(),
        det.hid_usage_page,
        det.vid,
        det.pid,
        det.bus,
        det.device.port_numbers().unwrap_or_default(),
    )
}

/// Open a shared HID backend with self-healing reopener, given the raw USB
/// coordinates of the device. Used by driver `open()` implementations.
pub fn open_hid_with_reopener(
    device: rusb::Device<GlobalContext>,
    usage_page: Option<u16>,
    vid: u16,
    pid: u16,
    bus: u8,
    port_numbers: Vec<u8>,
) -> Result<Arc<Mutex<RusbHid>>> {
    open_with_retry(&device, || {
        let transport = RusbHid::open_by_usage_with_reopener(
            device.clone(),
            usage_page,
            make_rusb_reopener(vid, pid, bus, port_numbers.clone(), usage_page),
        )?;
        Ok(Arc::new(Mutex::new(transport)))
    })
}

/// Open the primary USB-bulk transport for a detected device, with retry
/// logic and a USB reset on the second-to-last attempt.
pub fn open_usb_bulk_backend(det: &DetectedDevice) -> Result<RusbBulk> {
    open_with_retry(&det.device, || {
        let mut transport = RusbBulk::open_device(det.device.clone())?;
        transport.detach_and_configure(det.name)?;
        Ok(transport)
    })
}
