use anyhow::Result;
use hidapi::HidApi;
use lianli_shared::device_id::{uses_hid, DeviceFamily, UsbId, KNOWN_DEVICES};
use lianli_transport::{HidBackend, HidBackendKind, HidReopener, RusbHidTransport};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Build a reopener that re-acquires the same HID device via hidapi by VID/PID.
/// Used to recover from stale handles after USB suspend/resume.
fn make_hidapi_reopener(vid: u16, pid: u16, family: DeviceFamily) -> HidReopener {
    Arc::new(move || {
        let api = HidApi::new().map_err(|e| anyhow::anyhow!("hidapi init: {e}"))?;
        let det = find_hid_devices_by_family(&api, family)
            .into_iter()
            .find(|d| d.vid == vid && d.pid == pid)
            .ok_or_else(|| {
                anyhow::anyhow!("HID device {vid:04x}:{pid:04x} not enumerable on reopen")
            })?;
        let dev = api
            .open_path(&det.path)
            .map_err(|e| anyhow::anyhow!("hidapi open_path: {e}"))?;
        Ok(HidBackendKind::Hidapi(dev))
    })
}

/// Build a reopener that re-acquires the same HID device via the rusb backend.
fn make_rusb_reopener(vid: u16, pid: u16, usage_page: Option<u16>) -> HidReopener {
    Arc::new(move || {
        let usb_dev = find_usb_device(vid, pid).ok_or_else(|| {
            anyhow::anyhow!("USB device {vid:04x}:{pid:04x} not enumerable on reopen")
        })?;
        let transport = RusbHidTransport::open_by_usage(usb_dev, usage_page)
            .map_err(|e| anyhow::anyhow!("rusb hid open: {e}"))?;
        Ok(HidBackendKind::Rusb(transport))
    })
}

/// A detected USB device with its identified family.
#[derive(Debug)]
pub struct DetectedDevice {
    pub device: Device<GlobalContext>,
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
    /// Stable device ID: serial if unique, otherwise USB port path (bus-port topology).
    pub fn device_id(&self) -> String {
        match &self.serial {
            Some(s) if !is_non_unique_serial(s.as_str()) => {
                format!("hid:{}", s)
            }
            _ => {
                let port_path = self
                    .device
                    .port_numbers()
                    .ok()
                    .filter(|p| !p.is_empty())
                    .map(|ports| {
                        let parts: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
                        format!("{}-{}", self.bus, parts.join("."))
                    })
                    .unwrap_or_else(|| format!("{}-{}", self.bus, self.address));
                format!("hid:{:04x}:{:04x}:{}", self.vid, self.pid, port_path)
            }
        }
    }
}

/// A detected HID device with its identified family.
#[derive(Debug, Clone)]
pub struct DetectedHidDevice {
    pub family: DeviceFamily,
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    pub path: std::ffi::CString,
    pub serial: Option<String>,
    /// USB port path (e.g. "1-5.3") for stable device IDs.
    pub usb_port_path: Option<String>,
}

/// Known non-unique HID serial strings (chip manufacturer names, firmware
/// version markers, etc. — not actual per-device serials).
const NON_UNIQUE_SERIALS: &[&str] = &["Nuvoton"];

fn is_non_unique_serial(s: &str) -> bool {
    NON_UNIQUE_SERIALS.contains(&s) || s.starts_with("TL_LCDV")
}

impl DetectedHidDevice {
    /// Stable device ID: serial if unique, otherwise USB port path.
    pub fn device_id(&self) -> String {
        match &self.serial {
            Some(s) if !is_non_unique_serial(s.as_str()) => {
                format!("hid:{}", s)
            }
            _ => match &self.usb_port_path {
                Some(port_path) => {
                    format!("hid:{:04x}:{:04x}:{}", self.vid, self.pid, port_path)
                }
                None => {
                    // Fallback: hash the HID path
                    let path_bytes = self.path.as_bytes();
                    let hash: u32 = path_bytes
                        .iter()
                        .fold(0u32, |acc, &b| acc.wrapping_mul(31).wrapping_add(b as u32));
                    format!("hid:{:04x}:{:04x}:{:04x}", self.vid, self.pid, hash as u16)
                }
            },
        }
    }
}

/// Look up the USB port path for a device by VID/PID.
/// Returns e.g. "1-5.3" (bus-port topology), stable across reboots.
fn usb_port_path(vid: u16, pid: u16) -> Option<String> {
    let devices = rusb::devices().ok()?;
    for device in devices.iter() {
        let desc = device.device_descriptor().ok()?;
        if desc.vendor_id() == vid && desc.product_id() == pid {
            let bus = device.bus_number();
            let ports = device.port_numbers().ok()?;
            if !ports.is_empty() {
                let parts: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
                return Some(format!("{}-{}", bus, parts.join(".")));
            }
        }
    }
    None
}

/// Enumerate all Lian Li USB devices on the system.
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

            // Try to read serial number
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

/// Find all USB devices matching a specific family.
pub fn find_devices_by_family(family: DeviceFamily) -> Result<Vec<DetectedDevice>> {
    Ok(enumerate_devices()?
        .into_iter()
        .filter(|d| d.family == family)
        .collect())
}

/// Enumerate all known Lian Li HID devices.
///
/// When a device entry specifies `hid_usage_page`, only the HID interface
/// matching that usage page is returned. Otherwise, deduplicates by
/// vid:pid:serial to avoid opening the wrong interface.
pub fn enumerate_hid_devices(api: &HidApi) -> Vec<DetectedHidDevice> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();

    for dev_info in api.device_list() {
        let vid = dev_info.vendor_id();
        let pid = dev_info.product_id();
        let id = UsbId::new(vid, pid);

        if let Some(entry) = KNOWN_DEVICES.iter().find(|e| e.id == id) {
            if !uses_hid(entry.family) {
                continue;
            }

            // Filter by usage page if the device requires a specific HID interface
            if let Some(required_page) = entry.hid_usage_page {
                if dev_info.usage_page() != required_page {
                    continue;
                }
            } else {
                // No usage page filter — deduplicate by vid:pid:serial
                // to avoid opening the wrong HID interface.
                // (TL Fan uses usage_page filtering so this branch only
                // applies to ENE and other devices with unique serials.)
                let serial_str = dev_info.serial_number().unwrap_or("").to_string();
                let dedup_key = (vid, pid, serial_str);
                if !seen.insert(dedup_key) {
                    continue;
                }
            }

            let serial = dev_info.serial_number().map(|s| s.to_string());

            debug!(
                "Found HID {} ({:04x}:{:04x}) usage_page={:#06x} path={:?} serial={:?}",
                entry.name,
                vid,
                pid,
                dev_info.usage_page(),
                dev_info.path(),
                serial
            );

            found.push(DetectedHidDevice {
                family: entry.family,
                name: entry.name,
                vid,
                pid,
                path: dev_info.path().to_owned(),
                usb_port_path: usb_port_path(vid, pid),
                serial,
            });
        }
    }

    found
}

/// Find HID devices matching a specific family.
pub fn find_hid_devices_by_family(api: &HidApi, family: DeviceFamily) -> Vec<DetectedHidDevice> {
    enumerate_hid_devices(api)
        .into_iter()
        .filter(|d| d.family == family)
        .collect()
}

/// Open a detected HID device as an LCD controller via hidapi.
pub fn open_hid_lcd_device(
    api: &HidApi,
    det: &DetectedHidDevice,
) -> Option<Result<Box<dyn crate::traits::LcdDevice>>> {
    let pid = det.pid;
    match det.family {
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => {
            Some(open_hidapi_with_retry(api, det, |backend| {
                let backend = Arc::new(Mutex::new(backend));
                crate::hydroshift_lcd::HydroShiftLcdController::new(backend, pid)
                    .map(|d| Box::new(d) as Box<dyn crate::traits::LcdDevice>)
            }))
        }
        DeviceFamily::TlLcd => Some(open_hidapi_with_retry(api, det, |backend| {
            let backend = Arc::new(Mutex::new(backend));
            let mut tl = crate::tl_lcd::TlLcdDevice::new(backend);
            crate::traits::LcdDevice::initialize(&mut tl)?;
            Ok(Box::new(tl) as Box<dyn crate::traits::LcdDevice>)
        })),
        _ => None,
    }
}

/// Open a HID LCD device by VID/PID using hidapi with retry logic.
///
/// Unlike `open_hid_lcd_device` (which requires a pre-enumerated `DetectedHidDevice`),
/// this function handles the case where no hidraw node exists yet by performing
/// USB reset + re-enumeration before retrying.
pub fn open_hid_lcd_by_vid_pid(
    vid: u16,
    pid: u16,
    family: DeviceFamily,
) -> Result<Box<dyn crate::traits::LcdDevice>> {
    let usb_device = find_usb_device(vid, pid);

    for attempt in 0..=3u32 {
        let api = HidApi::new().map_err(|e| anyhow::anyhow!("hidapi init: {e}"))?;
        let hid_devs = find_hid_devices_by_family(&api, family);

        if let Some(det) = hid_devs.into_iter().next() {
            match open_hid_lcd_device(&api, &det) {
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
                None => {
                    return Err(anyhow::anyhow!("family does not support LCD"));
                }
            }
        } else if attempt < 3 {
            warn!(
                "No hidraw node for {:04x}:{:04x} (attempt {}), resetting USB",
                vid,
                pid,
                attempt + 1
            );
        } else {
            return Err(anyhow::anyhow!(
                "no HID device found for {vid:04x}:{pid:04x} after 4 attempts"
            ));
        }

        if let Some(ref usb_dev) = usb_device {
            let _ = RusbHidTransport::reset_usb_device(usb_dev);
            std::thread::sleep(Duration::from_secs(3));
        } else {
            return Err(anyhow::anyhow!(
                "no USB device found for reset ({vid:04x}:{pid:04x})"
            ));
        }
    }
    unreachable!()
}

/// Ensure all known HID devices have hidraw nodes by performing USB resets
/// on devices the kernel failed to bind. Some devices persist malformed HID
/// report descriptors across reboots; a USB port reset restores them.
pub fn ensure_hid_devices_bound() {
    let usb_hid_devices: Vec<(u16, u16, &str)> = match enumerate_devices() {
        Ok(devs) => devs
            .into_iter()
            .filter(|d| uses_hid(d.family))
            .map(|d| (d.vid, d.pid, d.name))
            .collect(),
        Err(_) => return,
    };

    if usb_hid_devices.is_empty() {
        return;
    }

    let api = match HidApi::new() {
        Ok(api) => api,
        Err(_) => return,
    };

    let hid_vids_pids: HashSet<(u16, u16)> = api
        .device_list()
        .map(|d| (d.vendor_id(), d.product_id()))
        .collect();

    let mut reset_count = 0u32;
    for (vid, pid, name) in &usb_hid_devices {
        if hid_vids_pids.contains(&(*vid, *pid)) {
            continue;
        }
        info!("No hidraw node for {name} ({vid:04x}:{pid:04x}), performing USB reset");
        let dev = rusb::devices().ok().and_then(|devs| {
            devs.iter().find(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == *vid && desc.product_id() == *pid)
                    .unwrap_or(false)
            })
        });
        if let Some(usb_dev) = dev {
            if device_responds_to_descriptor(&usb_dev) {
                nudge_kernel_to_bind_hid(&usb_dev);
                if poll_for_hidraw(*vid, *pid, Duration::from_millis(2000)) {
                    info!("{name}: hidraw appeared after kernel bind, no reset needed");
                    continue;
                }
                info!("{name}: hidraw still missing after wait, falling through to reset");
            }
            match RusbHidTransport::reset_usb_device(&usb_dev) {
                Ok(()) => {
                    info!("USB reset successful for {name}");
                    reset_count += 1;
                }
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("Entity not found") {
                        info!("USB reset successful for {name} (device re-enumerated)");
                        reset_count += 1;
                    } else {
                        warn!("USB reset failed for {name}: {e}");
                    }
                }
            }
        }
    }

    if reset_count > 0 {
        info!("Waiting 3s for {reset_count} device(s) to re-enumerate after USB reset");
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

fn nudge_kernel_to_bind_hid(device: &Device<GlobalContext>) {
    let Ok(handle) = device.open() else { return };
    let Ok(config) = device.active_config_descriptor() else {
        return;
    };
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            if desc.class_code() == 0x03 {
                let _ = handle.attach_kernel_driver(desc.interface_number());
            }
        }
    }
}

fn poll_for_hidraw(vid: u16, pid: u16, max_wait: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < max_wait {
        if let Ok(api) = HidApi::new() {
            if api
                .device_list()
                .any(|d| d.vendor_id() == vid && d.product_id() == pid)
            {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn device_responds_to_descriptor(device: &Device<GlobalContext>) -> bool {
    let Ok(desc) = device.device_descriptor() else {
        return false;
    };
    let Ok(handle) = device.open() else {
        return false;
    };
    let langs = match handle.read_languages(Duration::from_millis(250)) {
        Ok(l) if !l.is_empty() => l,
        _ => return desc.product_string_index().is_some(),
    };
    if let Some(_idx) = desc.product_string_index() {
        return handle
            .read_product_string(langs[0], &desc, Duration::from_millis(250))
            .is_ok();
    }
    true
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
                        let _ = RusbHidTransport::reset_usb_device(usb_dev);
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

/// Find the rusb `Device` matching a VID/PID pair.
fn find_usb_device(vid: u16, pid: u16) -> Option<Device<GlobalContext>> {
    rusb::devices().ok()?.iter().find(|d| {
        d.device_descriptor()
            .map(|desc| desc.vendor_id() == vid && desc.product_id() == pid)
            .unwrap_or(false)
    })
}

/// Open a detected HID device as an LCD controller via rusb.
pub fn open_hid_lcd_device_rusb(
    det: &DetectedDevice,
) -> Option<Result<Box<dyn crate::traits::LcdDevice>>> {
    match det.family {
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => {
            let pid = det.pid;
            Some(open_with_retry(&det.device, || {
                let transport =
                    RusbHidTransport::open_by_usage(det.device.clone(), det.hid_usage_page)?;
                let mut backend = HidBackend::from_rusb(transport)
                    .with_reopener(make_rusb_reopener(det.vid, det.pid, det.hid_usage_page));
                backend.read_flush();
                let backend = Arc::new(Mutex::new(backend));
                crate::hydroshift_lcd::HydroShiftLcdController::new(backend, pid)
                    .map(|d| Box::new(d) as Box<dyn crate::traits::LcdDevice>)
            }))
        }
        DeviceFamily::TlLcd => Some(open_with_retry(&det.device, || {
            let transport =
                RusbHidTransport::open_by_usage(det.device.clone(), det.hid_usage_page)?;
            let backend = HidBackend::from_rusb(transport).with_reopener(make_rusb_reopener(
                det.vid,
                det.pid,
                det.hid_usage_page,
            ));
            let backend = Arc::new(Mutex::new(backend));
            let mut tl = crate::tl_lcd::TlLcdDevice::new(backend);
            crate::traits::LcdDevice::initialize(&mut tl)?;
            Ok(Box::new(tl) as Box<dyn crate::traits::LcdDevice>)
        })),
        _ => None,
    }
}

/// Wrap hidapi open with retry logic. On failure, performs USB reset and retries.
pub fn open_hidapi_with_retry<T>(
    api: &HidApi,
    det: &DetectedHidDevice,
    mut create_fn: impl FnMut(HidBackend) -> Result<T>,
) -> Result<T> {
    let usb_device = find_usb_device(det.vid, det.pid);
    let label = format!("HID open {} ({:04x}:{:04x})", det.name, det.vid, det.pid);

    let backend = try_open_with_retry(usb_device.as_ref(), &label, || {
        let hid_dev = api
            .open_path(&det.path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut backend = HidBackend::from_hidapi(hid_dev)
            .with_reopener(make_hidapi_reopener(det.vid, det.pid, det.family));
        backend.read_flush();
        Ok(backend)
    })?;
    create_fn(backend)
}

/// Open a shared HID backend via hidapi with retry logic.
/// Returns an `Arc<Mutex<HidBackend>>` that can be shared between multiple controllers.
pub fn open_hid_backend_hidapi(
    api: &HidApi,
    det: &DetectedHidDevice,
) -> Result<Arc<Mutex<HidBackend>>> {
    open_hidapi_with_retry(api, det, |backend| Ok(Arc::new(Mutex::new(backend))))
}

/// Open a shared HID backend via rusb with retry logic.
/// Returns an `Arc<Mutex<HidBackend>>` that can be shared between multiple controllers.
pub fn open_hid_backend_rusb(det: &DetectedDevice) -> Result<Arc<Mutex<HidBackend>>> {
    let vid = det.vid;
    let pid = det.pid;
    let usage_page = det.hid_usage_page;
    open_with_retry(&det.device, || {
        let transport = RusbHidTransport::open_by_usage(det.device.clone(), det.hid_usage_page)?;
        let backend = HidBackend::from_rusb(transport)
            .with_reopener(make_rusb_reopener(vid, pid, usage_page));
        Ok(Arc::new(Mutex::new(backend)))
    })
}

/// Result of initializing a wired HID controller that may provide fan, RGB, or both.
pub struct WiredControllerSet {
    pub fan: Option<Box<dyn crate::traits::FanDevice>>,
    /// RGB devices as `(suffix, device)` pairs. Suffix is empty for single-zone devices,
    /// or "portN" for multi-port devices like TL Fan.
    pub rgb: Vec<(String, Box<dyn crate::traits::RgbDevice>)>,
}

/// Create all controllers (fan + RGB) for a device family in a single init pass.
/// This avoids double-initialization for devices that support both fan and RGB
/// by creating one controller and sharing it via `Arc`.
pub fn create_wired_controllers(
    family: DeviceFamily,
    pid: u16,
    backend: Arc<Mutex<HidBackend>>,
) -> Option<Result<WiredControllerSet>> {
    match family {
        DeviceFamily::TlFan => Some(crate::tl_fan::TlFanController::new(backend).map(|ctrl| {
            let ctrl = Arc::new(ctrl);
            let rgb: Vec<_> = ctrl
                .port_devices()
                .into_iter()
                .map(|(port, dev)| {
                    (
                        format!("port{port}"),
                        Box::new(dev) as Box<dyn crate::traits::RgbDevice>,
                    )
                })
                .collect();
            WiredControllerSet {
                fan: Some(Box::new(ctrl)),
                rgb,
            }
        })),
        DeviceFamily::Ene6k77 => Some(crate::ene6k77::Ene6k77Controller::new(backend, pid).map(
            |ctrl| {
                let ctrl = Arc::new(ctrl);
                let rgb: Vec<_> = ctrl
                    .group_devices()
                    .into_iter()
                    .map(|(group, dev)| {
                        (
                            format!("group{group}"),
                            Box::new(dev) as Box<dyn crate::traits::RgbDevice>,
                        )
                    })
                    .collect();
                WiredControllerSet {
                    fan: Some(Box::new(Arc::clone(&ctrl))),
                    rgb,
                }
            },
        )),
        DeviceFamily::Galahad2Trinity => Some(
            crate::galahad2_trinity::Galahad2TrinityController::new(backend, pid).map(|c| {
                WiredControllerSet {
                    fan: None,
                    rgb: vec![(
                        String::new(),
                        Box::new(c) as Box<dyn crate::traits::RgbDevice>,
                    )],
                }
            }),
        ),
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => Some(
            crate::hydroshift_lcd::HydroShiftLcdController::new(Arc::clone(&backend), pid)
                .and_then(|lcd_ctrl| {
                    let rgb_ctrl = crate::hydroshift_lcd::AioLcdRgbController::new(backend, pid)?;
                    Ok(WiredControllerSet {
                        fan: Some(Box::new(Arc::new(lcd_ctrl))),
                        rgb: vec![(
                            String::new(),
                            Box::new(rgb_ctrl) as Box<dyn crate::traits::RgbDevice>,
                        )],
                    })
                }),
        ),
        _ => None,
    }
}

/// Create an HID LCD controller from a pre-opened shared backend.
pub fn create_hid_lcd_device(
    family: DeviceFamily,
    pid: u16,
    backend: Arc<Mutex<HidBackend>>,
) -> Option<Result<Box<dyn crate::traits::LcdDevice>>> {
    match family {
        DeviceFamily::HydroShiftLcd | DeviceFamily::Galahad2Lcd => Some(
            crate::hydroshift_lcd::HydroShiftLcdController::new(backend, pid)
                .map(|d| Box::new(d) as Box<dyn crate::traits::LcdDevice>),
        ),
        DeviceFamily::TlLcd => {
            let mut tl = crate::tl_lcd::TlLcdDevice::new(backend);
            Some(
                crate::traits::LcdDevice::initialize(&mut tl)
                    .map(|_| Box::new(tl) as Box<dyn crate::traits::LcdDevice>),
            )
        }
        _ => None,
    }
}

/// Find LCD devices (SLV3/TLV2 wireless LCD fans via USB bulk).
pub fn find_wireless_lcd_devices() -> Result<Vec<Device<GlobalContext>>> {
    let devices = rusb::devices()?;
    let lcd_pids: &[(u16, u16)] = &[
        (0x1CBE, 0x0005), // SLV3
        (0x1CBE, 0x0006), // TLV2
    ];
    let mut list = Vec::new();
    for device in devices.iter() {
        if let Ok(desc) = device.device_descriptor() {
            let vid = desc.vendor_id();
            let pid = desc.product_id();
            if lcd_pids.iter().any(|&(v, p)| v == vid && p == pid) {
                list.push(device);
            }
        }
    }
    list.sort_by_key(|dev| (dev.bus_number(), dev.address()));
    Ok(list)
}
