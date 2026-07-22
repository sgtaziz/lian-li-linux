//! Thermal alert subsystem — overrides RGB lighting when CPU/GPU temperature
//! exceeds a configured threshold.
//!
//! Architecture: a background thread polls CPU/GPU sensors every 1 second.
//! When temp >= threshold, it pushes an override color to the shared state.
//! The RGB controller checks the override before applying user effects — when
//! active, it sends Static mode with the alert color instead.

use lianli_shared::config::ThermalAlertSettings;
use lianli_shared::sensors::{
    enumerate_sensors, pick_source_for_category, read_sensor_value, resolve_sensor, SensorCategory,
    SensorSource,
};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Shared thermal-alert state. The RGB controller reads `override_color`
/// to decide whether to apply the alert override.
pub type SharedThermalAlert = Arc<Mutex<Option<[u8; 3]>>>;

pub fn new_shared() -> SharedThermalAlert {
    Arc::new(Mutex::new(None))
}

pub struct ThermalAlertMonitor {
    settings: Arc<Mutex<ThermalAlertSettings>>,
    override_color: SharedThermalAlert,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThermalAlertMonitor {
    pub fn new(settings: ThermalAlertSettings) -> Self {
        Self {
            settings: Arc::new(Mutex::new(settings)),
            override_color: new_shared(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn shared_override(&self) -> SharedThermalAlert {
        Arc::clone(&self.override_color)
    }

    #[allow(dead_code)]
    pub fn update_settings(&self, settings: ThermalAlertSettings) {
        *self.settings.lock() = settings;
    }

    pub fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let settings = Arc::clone(&self.settings);
        let override_color = Arc::clone(&self.override_color);
        let stop = Arc::clone(&self.stop_flag);
        self.thread = Some(thread::spawn(move || run(settings, override_color, stop)));
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        *self.override_color.lock() = None;
    }
}

impl Drop for ThermalAlertMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    settings: Arc<Mutex<ThermalAlertSettings>>,
    override_color: SharedThermalAlert,
    stop_flag: Arc<AtomicBool>,
) {
    let sensors = enumerate_sensors();
    let cpu_source = pick_source_for_category(SensorCategory::CpuTemp, &sensors);
    let gpu_source = pick_source_for_category(SensorCategory::GpuTemp, &sensors);

    let mut cpu_triggered;
    let mut gpu_triggered;

    info!("Thermal alert monitor started");
    while !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(POLL_INTERVAL);
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let cfg = settings.lock().clone();

        let cpu_temp = cpu_source
            .as_ref()
            .map(|s| s.to_sensor_source())
            .and_then(|s| read_temp(&s));
        let gpu_temp = gpu_source
            .as_ref()
            .map(|s| s.to_sensor_source())
            .and_then(|s| read_temp(&s));
        // Evaluate trigger state
        cpu_triggered =
            cfg.cpu.enabled && cpu_temp.map_or(false, |t| t >= cfg.cpu.threshold as f32);
        gpu_triggered =
            cfg.gpu.enabled && gpu_temp.map_or(false, |t| t >= cfg.gpu.threshold as f32);

        // Determine override color: GPU takes priority if both triggered
        // (last-pushed wins behaviour)
        let new_override = if gpu_triggered {
            Some(cfg.gpu.alert_color)
        } else if cpu_triggered {
            Some(cfg.cpu.alert_color)
        } else {
            None
        };

        let mut guard = override_color.lock();
        if *guard != new_override {
            match new_override {
                Some(color) => {
                    info!(
                        "Thermal alert triggered: CPU={:?}°C GPU={:?}°C — overriding RGB with [{},{},{}]",
                        cpu_temp.map(|t| t as i32),
                        gpu_temp.map(|t| t as i32),
                        color[0], color[1], color[2]
                    );
                }
                None => {
                    info!("Thermal alert cleared — restoring RGB");
                }
            }
            *guard = new_override;
        }
    }

    *override_color.lock() = None;
    debug!("Thermal alert monitor stopped");
}

fn read_temp(source: &SensorSource) -> Option<f32> {
    let resolved = resolve_sensor(source, 1)?;
    read_sensor_value(&resolved).ok()
}
