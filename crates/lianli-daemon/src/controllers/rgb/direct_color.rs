use super::RgbController;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::debug;

/// Buffers per-device, per-zone direct color updates for async flushing.
///
/// The OpenRGB TCP handler writes latest colors here (fast, no device I/O).
/// A writer thread flushes dirty devices at ~30fps, dropping intermediate frames.
pub struct DirectColorBuffer {
    pending: HashMap<String, HashMap<u8, Vec<[u8; 3]>>>,
}

impl DirectColorBuffer {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Store colors for a device zone (overwrites any previous pending value).
    pub fn set(&mut self, device_id: String, zone: u8, colors: Vec<[u8; 3]>) {
        self.pending
            .entry(device_id)
            .or_default()
            .insert(zone, colors);
    }

    /// Take all pending updates, clearing the buffer.
    pub fn take_all(&mut self) -> HashMap<String, HashMap<u8, Vec<[u8; 3]>>> {
        std::mem::take(&mut self.pending)
    }
}

/// Spawns a background thread that flushes buffered direct colors.
///
/// Wired devices are processed first for lowest latency.
/// Wireless devices use single-frame direct sends.
pub fn start_direct_color_writer(
    rgb: Arc<Mutex<RgbController>>,
    buffer: Arc<Mutex<DirectColorBuffer>>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        debug!("Direct color writer started");

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let updates = buffer.lock().take_all();

            if !updates.is_empty() {
                let mut wired = Vec::new();
                let mut wireless = Vec::new();
                {
                    let rgb = rgb.lock();
                    for (device_id, zones) in updates {
                        if rgb.is_wireless(&device_id) {
                            wireless.push((device_id, zones));
                        } else {
                            wired.push((device_id, zones));
                        }
                    }
                }

                if !wired.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wired {
                        for (zone, colors) in zones {
                            if let Err(e) = rgb.set_direct_colors(&device_id, zone, &colors) {
                                debug!("Wired flush error for {device_id} zone {zone}: {e}");
                            }
                        }
                    }
                }

                if !wireless.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wireless {
                        for (zone, colors) in zones {
                            if let Err(e) = rgb.set_direct_colors(&device_id, zone, &colors) {
                                debug!("Wireless flush error for {device_id} zone {zone}: {e}");
                            }
                        }
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }

        debug!("Direct color writer stopped");
    })
}
