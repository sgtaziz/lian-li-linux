//! Grouped sub-structs extracted from [`super::ServiceManager`] to keep its
//! field count manageable.
//!
//! Each sub-struct owns a cohesive cluster of fields and exposes the small
//! set of operations they support. `ServiceManager` holds one of each as a
//! plain field — `self.ipc.state`, `self.openrgb.thread`, etc.
//!
//! Pattern: extract → update `ServiceManager::new` → fix `self.X` call sites.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;

use crate::controllers::rgb::{DirectColorBuffer, RgbController};
use crate::ipc::DaemonState;
use crate::openrgb_server;
use lianli_shared::ipc::DeviceInfo;
use lianli_transport::RusbHid;

// ──────────────────────────────────────────────────────────────────────
// IPC subsystem
// ──────────────────────────────────────────────────────────────────────

/// IPC server lifecycle: the shared `DaemonState`, a stop flag for the
/// connection-accept thread, and the thread handle itself.
pub struct IpcSubsystem {
    pub state: Arc<Mutex<DaemonState>>,
    pub stop: Arc<AtomicBool>,
    pub thread: Option<JoinHandle<()>>,
}

impl IpcSubsystem {
    pub fn new(state: Arc<Mutex<DaemonState>>) -> Self {
        Self {
            state,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    /// Stop the IPC thread and join it. Safe to call multiple times.
    pub fn shutdown(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// OpenRGB subsystem
// ──────────────────────────────────────────────────────────────────────

/// OpenRGB SDK server lifecycle.
pub struct OpenRgbSubsystem {
    pub state: Arc<Mutex<openrgb_server::OpenRgbServerState>>,
    pub stop: Arc<AtomicBool>,
    pub thread: Option<JoinHandle<()>>,
}

impl OpenRgbSubsystem {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(openrgb_server::OpenRgbServerState::default())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    /// Stop the OpenRGB thread and join it.
    pub fn shutdown(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Default for OpenRgbSubsystem {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────────────────────────────────────────────────
// Controller host
// ──────────────────────────────────────────────────────────────────────

use crate::controllers::aio::AioController;
use crate::controllers::fan::FanController;

/// Background controllers (fan curve evaluation, AIO pump control, RGB
/// effects) and the per-device direct-color flush thread.
pub struct Controllers {
    pub fan: Option<FanController>,
    pub aio: Option<AioController>,
    pub rgb: Option<Arc<Mutex<RgbController>>>,
    pub thermal_alert: Option<crate::thermal_alert::ThermalAlertMonitor>,
    pub direct_color_buffer: Arc<Mutex<DirectColorBuffer>>,
    pub direct_color_writer: Option<JoinHandle<()>>,
}

impl Controllers {
    pub fn new() -> Self {
        Self {
            fan: None,
            aio: None,
            rgb: None,
            thermal_alert: None,
            direct_color_buffer: Arc::new(Mutex::new(DirectColorBuffer::new())),
            direct_color_writer: None,
        }
    }

    /// Stop every controller and join its thread.
    pub fn shutdown(&mut self) {
        if let Some(fan) = self.fan.take() {
            fan.stop();
        }
        if let Some(aio) = self.aio.take() {
            aio.stop();
        }
        if let Some(mut ta) = self.thermal_alert.take() {
            ta.stop();
        }
        if let Some(writer) = self.direct_color_writer.take() {
            let _ = writer.join();
        }
        // RgbController has no stop() — its threads live inside
        // DirectColorBuffer / OpenRGB subsystem, both of which are
        // shut down separately.
        self.rgb.take();
    }
}

impl Default for Controllers {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────────────────────────────────────────────────
// Wired device registry
// ──────────────────────────────────────────────────────────────────────

use lianli_devices::traits::FanDevice;
use std::collections::{HashMap, HashSet};

/// Wired USB device registry: shared fan device handles, cached HID backends,
/// cached USB device list for IPC sync, hot-plug tracking, and TL LCD port
/// indices.
///
/// All five fields move together whenever a device is plugged or unplugged,
/// so keeping them in one struct makes the lifecycle obvious.
pub struct DeviceRegistry {
    /// Per-port `DeviceInfo` for wired fan devices (populated by init).
    pub fan_device_info: Vec<DeviceInfo>,
    /// Shared reference to wired fan device handles (for RPM reading).
    pub fan_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    /// Shared HID backends keyed by device ID — allows fan, RGB, and LCD
    /// controllers for the same physical device to share one USB handle.
    pub hid_backends: HashMap<String, Arc<Mutex<RusbHid>>>,
    /// Hot-plug detection: device IDs seen at the last topology scan.
    pub last_wired_ids: HashSet<String>,
    /// Cached USB device list from `enumerate_devices()` — refreshed every
    /// 10 s and surfaced to the GUI via `sync_ipc_state`.
    pub cached_usb_devices: Vec<DeviceInfo>,
    /// TL LCD `(port, fan_index)` per device_id. Probed once at init.
    pub tl_lcd_port_index: HashMap<String, (u8, u8)>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self {
            fan_device_info: Vec::new(),
            fan_devices: Arc::new(HashMap::new()),
            hid_backends: HashMap::new(),
            last_wired_ids: HashSet::new(),
            cached_usb_devices: Vec::new(),
            tl_lcd_port_index: HashMap::new(),
        }
    }

    /// Clear all device state (called on shutdown).
    pub fn clear(&mut self) {
        self.fan_device_info.clear();
        self.fan_devices = Arc::new(HashMap::new());
        self.hid_backends.clear();
        self.cached_usb_devices.clear();
        // Keep `last_wired_ids` and `tl_lcd_port_index` — they describe what
        // *should* be plugged in, not what currently is.
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}
