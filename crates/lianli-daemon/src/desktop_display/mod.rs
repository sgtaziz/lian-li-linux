//! Auto-attaches evdi virtual displays to connected Lian Li TURZX panels.
//!
//! For each `(VID=0x1A86, PID ∈ 0xAD10..0xAD3F)` device on the bus we run a
//! dedicated worker thread. The worker opens the USB panel via
//! [`TurzxDisplay`], spins up an evdi display node fed with the device's own
//! EDID, encodes framebuffer updates to H.264 via libavcodec, and pushes the
//! packets as TURZX stream A.

mod enumerate;
mod worker;

pub use enumerate::{enumerate_turzx, TurzxDeviceMatch};

use lianli_devices::turzx;
use lianli_media::video::ensure_ffmpeg_initialized;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use tracing::{info, warn};
use worker::spawn_worker;

/// Key identifying a single physical USB attachment (bus + address).
pub type DeviceKey = (u8, u8);

/// Handle to a running worker. Dropping it signals the worker to stop and
/// waits for it to join.
pub struct DesktopDisplayHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pid: u16,
}

impl Drop for DesktopDisplayHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            if let Err(e) = j.join() {
                warn!(
                    "TURZX {:04x}:{:04x} worker panicked on shutdown: {e:?}",
                    turzx::VID,
                    self.pid
                );
            }
        }
    }
}

/// Registry of running workers, keyed by USB (bus, address).
#[derive(Default)]
pub struct DesktopDisplayRegistry {
    inner: Mutex<HashMap<DeviceKey, DesktopDisplayHandle>>,
}

impl DesktopDisplayRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sync running workers against the currently-present TURZX devices.
    /// Spawns workers for new devices; drops handles (which stops + joins)
    /// for devices that have disappeared.
    pub fn sync(&self, present: &[TurzxDeviceMatch]) {
        let mut inner = self.inner.lock();
        let present_keys: std::collections::HashSet<DeviceKey> =
            present.iter().map(|m| m.key).collect();

        inner.retain(|key, _| {
            if present_keys.contains(key) {
                true
            } else {
                info!(
                    "TURZX {:02x}:{:02x} disappeared — stopping worker",
                    key.0, key.1
                );
                false
            }
        });

        for m in present {
            if inner.contains_key(&m.key) {
                continue;
            }
            match spawn_worker(m.pid) {
                Ok(h) => {
                    info!(
                        "TURZX {:04x}:{:04x} at bus {}/addr {} — worker spawned",
                        turzx::VID,
                        m.pid,
                        m.key.0,
                        m.key.1
                    );
                    inner.insert(m.key, h);
                }
                Err(e) => warn!(
                    "TURZX {:04x}:{:04x} at bus {}/addr {} — spawn failed: {e:#}",
                    turzx::VID,
                    m.pid,
                    m.key.0,
                    m.key.1
                ),
            }
        }
    }

    /// Stop (and join) any running worker for the given PID. Used when the
    /// user initiates a desktop→LCD mode switch on a device and we want the
    /// stream loop out of the way before the reboot flood.
    pub fn stop_for_pid(&self, pid: u16) {
        let mut inner = self.inner.lock();
        inner.retain(|_, h| {
            if h.pid == pid {
                info!(
                    "TURZX {:04x}:{:04x} — stopping worker for mode switch",
                    turzx::VID,
                    pid
                );
                false
            } else {
                true
            }
        });
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.lock();
        inner.clear();
        match lianli_evdi::remove_all_devices() {
            Ok(true) => info!("evdi virtual display nodes removed"),
            Ok(false) => {}
            Err(e) => warn!("remove_all_devices failed: {e:#}"),
        }
    }
}
