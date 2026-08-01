use lianli_shared::config::ThermalAlertSettings;
use lianli_shared::sensors::{
    enumerate_sensors, pick_source_for_category, read_sensor_value, resolve_sensor, SensorCategory,
    SensorSource,
};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const SENSOR_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AlertKind {
    Cpu,
    Gpu,
}

fn run(
    settings: Arc<Mutex<ThermalAlertSettings>>,
    override_color: SharedThermalAlert,
    stop_flag: Arc<AtomicBool>,
) {
    let mut sensors = enumerate_sensors();
    let mut cpu_source = pick_source_for_category(SensorCategory::CpuTemp, &sensors);
    let mut gpu_source = pick_source_for_category(SensorCategory::GpuTemp, &sensors);
    let mut last_sensor_refresh = Instant::now();

    let mut active_stack: VecDeque<AlertKind> = VecDeque::new();

    info!("Thermal alert monitor started");
    while !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(POLL_INTERVAL);
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        if last_sensor_refresh.elapsed() >= SENSOR_REFRESH_INTERVAL {
            sensors = enumerate_sensors();
            cpu_source = pick_source_for_category(SensorCategory::CpuTemp, &sensors);
            gpu_source = pick_source_for_category(SensorCategory::GpuTemp, &sensors);
            last_sensor_refresh = Instant::now();
            debug!("Thermal alert: refreshed sensor list");
        }

        let cfg = settings.lock().clone();

        let cpu_temp = cpu_source
            .as_ref()
            .map(|s| s.to_sensor_source())
            .and_then(|src| {
                let div = sensors
                    .iter()
                    .find(|si| si.source == src)
                    .map_or(1, |si| si.divider);
                read_temp(&src, div)
            });
        let gpu_temp = gpu_source
            .as_ref()
            .map(|s| s.to_sensor_source())
            .and_then(|src| {
                let div = sensors
                    .iter()
                    .find(|si| si.source == src)
                    .map_or(1, |si| si.divider);
                read_temp(&src, div)
            });

        let cpu_triggered =
            cfg.cpu.enabled && cpu_temp.map_or(false, |t| t >= cfg.cpu.threshold as f32);
        let gpu_triggered =
            cfg.gpu.enabled && gpu_temp.map_or(false, |t| t >= cfg.gpu.threshold as f32);

        let prev_len = active_stack.len();
        if cpu_triggered {
            if !active_stack.contains(&AlertKind::Cpu) {
                active_stack.push_back(AlertKind::Cpu);
            }
        } else {
            active_stack.retain(|&k| k != AlertKind::Cpu);
        }
        if gpu_triggered {
            if !active_stack.contains(&AlertKind::Gpu) {
                active_stack.push_back(AlertKind::Gpu);
            }
        } else {
            active_stack.retain(|&k| k != AlertKind::Gpu);
        }

        let new_override = active_stack.back().map(|&kind| match kind {
            AlertKind::Cpu => cfg.cpu.alert_color,
            AlertKind::Gpu => cfg.gpu.alert_color,
        });

        let mut guard = override_color.lock();
        if *guard != new_override || active_stack.len() != prev_len {
            match new_override {
                Some(color) => {
                    info!(
                        "Thermal alert active: CPU={:?}°C GPU={:?}°C — override [{},{},{}] (stack: {:?})",
                        cpu_temp.map(|t| t as i32),
                        gpu_temp.map(|t| t as i32),
                        color[0], color[1], color[2],
                        active_stack,
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

fn read_temp(source: &SensorSource, divider: usize) -> Option<f32> {
    let resolved = resolve_sensor(source, divider)?;
    read_sensor_value(&resolved).ok()
}
