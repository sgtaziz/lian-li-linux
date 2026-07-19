//! Grouped sub-structs extracted from [`super::ServiceManager`] to keep its
//! field count manageable.
//!
//! Each sub-struct owns a cohesive cluster of fields and exposes the small
//! set of operations they support. `ServiceManager` holds one of each as a
//! plain field — `self.ipc.state`, `self.openrgb.thread`, etc.
//!
//! Pattern: extract → update `ServiceManager::new` → fix `self.X` call sites.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;

use crate::ipc_server::DaemonState;
use crate::openrgb_server;
use crate::rgb_controller::{DirectColorBuffer, RgbController};
use crate::service::DaemonEvent;

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

use crate::aio_controller::AioController;
use crate::fan_controller::FanController;

/// Background controllers (fan curve evaluation, AIO pump control, RGB
/// effects) and the per-device direct-color flush thread.
pub struct Controllers {
    pub fan: Option<FanController>,
    pub aio: Option<AioController>,
    pub rgb: Option<Arc<Mutex<RgbController>>>,
    pub direct_color_buffer: Arc<Mutex<DirectColorBuffer>>,
    pub direct_color_writer: Option<JoinHandle<()>>,
}

impl Controllers {
    pub fn new() -> Self {
        Self {
            fan: None,
            aio: None,
            rgb: None,
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

// Suppress unused-import warning for Sender<DaemonEvent>; it's part of the
// public surface for handlers that send events.
#[allow(dead_code)]
fn _suppress_sender(_: Sender<DaemonEvent>) {}
