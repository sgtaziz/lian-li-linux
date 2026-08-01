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
    let busy_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(HashMap::new()));

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
                        if device_id.starts_with("wireless:") {
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
                    let flag = {
                        let mut flags = busy_flags.lock();
                        flags
                            .entry(device_id.clone())
                            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                            .clone()
                    };
                    if flag.swap(true, Ordering::Relaxed) {
                        continue;
                    }
                    thread::spawn(move || {
                        for (zone, colors) in &zones {
                            if let Err(e) = dev.set_direct_colors(*zone, colors) {
                                debug!("Wired flush error for {device_id} zone {zone}: {e}");
                            }
                        }
                        flag.store(false, Ordering::Relaxed);
                    });
                }
            } else {
                thread::sleep(Duration::from_millis(2));
            }
        }

        debug!("Direct color writer stopped");
    })
}
