//! Device driver registry — the single dispatch point for opening devices.
//!
//! Each USB-attached device family is represented by a [`DeviceDriver`] impl.
//! The daemon (or any other consumer) calls [`open_for_family`] with a
//! detected USB device and gets back a fully-initialised [`OpenedDevice`]
//! whose `fan` / `lcd` / `rgb` / `aio` slots are populated based on what the
//! family actually supports.
//!
//! Adding support for a new device = new module under `drivers/` + one line
//! in [`REGISTRY`]. The daemon never pattern-matches on `DeviceFamily` to
//! decide how to open a device.

use anyhow::Result;
use lianli_shared::device_id::{DeviceCapabilities, DeviceFamily, TransportKind};
use lianli_shared::id::DeviceId;
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;

use crate::traits::{AioDevice, FanDevice, LcdDevice, RgbDevice};

/// Context passed to [`DeviceDriver::open`]. Carries everything a driver needs
/// to locate, open, and claim its USB interface.
pub struct OpenContext {
    pub device: Device<GlobalContext>,
    pub family: DeviceFamily,
    pub vid: u16,
    pub pid: u16,
    pub bus: u8,
    pub address: u8,
    pub serial: Option<String>,
    pub hid_usage_page: Option<u16>,
}

impl OpenContext {
    /// Stable runtime identifier for this device.
    pub fn device_id(&self) -> DeviceId {
        match &self.serial {
            Some(s) if !s.is_empty() => DeviceId::wired(s),
            _ => DeviceId::wired(format!(
                "{:04x}:{:04x}:{}-{}",
                self.vid, self.pid, self.bus, self.address
            )),
        }
    }
}

/// A fully-initialised device, ready to be dispatched to fan/RGB/LCD/AIO
/// subsystems.
///
/// Each slot is `Option` because devices vary in what they expose:
/// - TL Fan controller fills `fan` + multiple `rgb` entries.
/// - TL LCD fills `lcd`.
/// - Galahad2 Trinity fills `fan` + `rgb` + `aio`.
/// - Universal Screen 8.8" LED ring fills `rgb` only.
pub struct OpenedDevice {
    pub id: DeviceId,
    pub family: DeviceFamily,
    pub capabilities: DeviceCapabilities,
    pub transport_kind: TransportKind,
    pub model_name: String,
    pub firmware: Option<String>,
    pub fan: Option<Box<dyn FanDevice>>,
    pub lcd: Option<Box<dyn LcdDevice>>,
    /// Multi-zone RGB devices: one entry per zone, labelled
    /// (`""` for single-zone, `"port0"`, `"group1"`, etc.).
    pub rgb: Vec<(String, std::sync::Arc<dyn RgbDevice>)>,
    pub aio: Option<Box<dyn AioDevice>>,
    /// For HID devices: the shared backend the controllers use, so the daemon
    /// can cache it and hand the same backend to a sibling LCD controller
    /// later. `None` for USB-bulk devices and LCD-only families.
    pub shared_hid: Option<SharedHid>,
}

impl OpenedDevice {
    /// `true` if this device exposes the given capability.
    pub fn has(&self, cap: DeviceCapabilities) -> bool {
        self.capabilities.contains(cap)
    }
}

/// Driver entry point. One implementation per device family.
///
/// Drivers are stateless — all per-device state lives in the controllers they
/// construct. The driver struct itself is just a marker so it can be referenced
/// from the static [`REGISTRY`].
pub trait DeviceDriver: Send + Sync {
    /// The family this driver handles.
    fn family(&self) -> DeviceFamily;

    /// Open the device, initialise it, and return the populated
    /// [`OpenedDevice`].
    fn open(&self, ctx: &OpenContext) -> Result<OpenedDevice>;
}

/// The static dispatch table. To add a device family: write a `Driver` struct
/// in a new module under `drivers/`, implement `DeviceDriver`, and add a
/// line here.
pub static REGISTRY: &[&dyn DeviceDriver] = &[
    &crate::ene6k77::Ene6k77Driver,
    &crate::tl_fan::TlFanDriver,
    &crate::tl_lcd::TlLcdDriver,
    &crate::galahad2_trinity::Galahad2TrinityDriver,
    &crate::hydroshift_lcd::HydroShiftLcdDriver,
    &crate::winusb::lcd::WinUsbLcdDriver,
    // Universal Screen 8.8" LED ring (0x0416:0x8050). Runtime RGB via the
    // 0x11 chunked-LED protocol (64-byte packets, 60 LEDs). Static + Direct.
    &crate::winusb::led::WinUsbLedDriver,
    &crate::strimer_plus::StrimerPlusDriver,
    &crate::winusb::wired_receiver::WiredReceiverDriver,
    &crate::winusb::hs2_oled_led::Hs2OledLedDriver,
];

/// Look up the driver for a given family.
pub fn driver_for_family(family: DeviceFamily) -> Option<&'static dyn DeviceDriver> {
    REGISTRY.iter().copied().find(|d| d.family() == family)
}

/// Convenience: open a detected device by delegating to its registered driver.
///
/// Returns `None` if no driver is registered for the family (e.g. RF dongles
/// and wireless-discovered devices don't go through this path).
pub fn open_device(family: DeviceFamily, ctx: &OpenContext) -> Option<Result<OpenedDevice>> {
    driver_for_family(family).map(|driver| driver.open(ctx))
}

/// Helper type for drivers whose controllers share an HID backend via
/// `Arc<Mutex<RusbHid>>`. Re-exported here so driver modules don't need to
/// spell out the full path.
pub type SharedHid = Arc<Mutex<lianli_transport::RusbHid>>;
