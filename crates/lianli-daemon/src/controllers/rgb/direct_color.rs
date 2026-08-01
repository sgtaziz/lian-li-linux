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
                let mut wired_tasks: Vec<(
                    Arc<dyn lianli_devices::traits::RgbDevice>,
                    String,
                    Vec<(u8, Vec<[u8; 3]>)>,
                )> = Vec::new();
                let mut wireless_updates: Vec<(String, HashMap<u8, Vec<[u8; 3]>>)> = Vec::new();

                {
                    let mut rgb = rgb.lock();
                    rgb.cache_direct_batch(&updates);
                    for (device_id, zones) in &updates {
                        if rgb.is_wireless(device_id) {
                            wireless_updates.push((device_id.clone(), zones.clone()));
                        } else if let Some(dev) = rgb.clone_wired_device(device_id) {
                            let zone_list: Vec<(u8, Vec<[u8; 3]>)> =
                                zones.iter().map(|(&z, c)| (z, c.clone())).collect();
                            wired_tasks.push((dev, device_id.clone(), zone_list));
                        }
                    }
                }

                std::thread::scope(|s| {
                    for (dev, device_id, zones) in &wired_tasks {
                        let device_id = device_id.clone();
                        let zones = zones.clone();
                        let dev = Arc::clone(dev);
                        s.spawn(move || {
                            for (zone, colors) in &zones {
                                if let Err(e) = dev.set_direct_colors(*zone, colors) {
                                    debug!("Wired flush error for {device_id} zone {zone}: {e}");
                                }
                            }
                        });
                    }
                });

                if !wireless_updates.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wireless_updates {
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
