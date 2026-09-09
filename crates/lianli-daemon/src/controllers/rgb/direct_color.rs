use super::RgbController;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::debug;

pub struct DirectColorBuffer {
    pending: HashMap<String, HashMap<u8, Vec<[u8; 3]>>>,
}

impl DirectColorBuffer {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    pub fn set(&mut self, device_id: String, zone: u8, colors: Vec<[u8; 3]>) {
        self.pending
            .entry(device_id)
            .or_default()
            .insert(zone, colors);
    }

    pub fn take_all(&mut self) -> HashMap<String, HashMap<u8, Vec<[u8; 3]>>> {
        std::mem::take(&mut self.pending)
    }
}
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
                let mut wireless = Vec::new();
                let mut wired = Vec::new();
                {
                    let mut rgb = rgb.lock();
                    rgb.cache_direct_batch(&updates);
                    for (device_id, zones) in &updates {
                        if rgb.software_controlled(device_id) {
                            wireless.push((device_id.clone(), zones.clone()));
                        } else if let Some(dev) = rgb.clone_wired_device(device_id) {
                            let z: Vec<_> = zones.iter().map(|(&k, v)| (k, v.clone())).collect();
                            wired.push((dev, device_id.clone(), z));
                        }
                    }
                }

                if !wireless.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wireless {
                        if let Err(e) = rgb.apply_wireless_batch(&device_id, &zones) {
                            debug!("Wireless flush error for {device_id}: {e}");
                        }
                    }
                }

                for (dev, device_id, zones) in wired {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    for (zone, colors) in zones {
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Err(e) = dev.set_direct_colors(zone, &colors) {
                            debug!("Wired flush error for {device_id} zone {zone}: {e}");
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(16));
        }

        debug!("Direct color writer stopped");
    })
}
