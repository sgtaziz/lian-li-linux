use crate::service::DaemonEvent;
use anyhow::{Context, Result};
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::{build_payload, SensorSnapshot, WirelessController};
use lianli_shared::fan::{interpolate_curve, FanConfig, FanCurve, FanSpeed};
use lianli_shared::sensors::{self, picker, ResolvedSensor, SensorInfo, SensorSource};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct FanController {
    config: FanConfig,
    curves: HashMap<String, FanCurve>,
    wireless: Option<Arc<WirelessController>>,
    wired_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    rgb_drift_enabled: bool,
    rgb_drift_interval: Duration,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    daemon_tx: Option<Sender<DaemonEvent>>,
}

impl FanController {
    pub fn new(
        config: FanConfig,
        curves: Vec<FanCurve>,
        wireless: Option<Arc<WirelessController>>,
        wired_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
        daemon_tx: Option<Sender<DaemonEvent>>,
        rgb_drift_enabled: bool,
        rgb_drift_interval: Duration,
    ) -> Self {
        let curves_map: HashMap<String, FanCurve> =
            curves.into_iter().map(|c| (c.name.clone(), c)).collect();

        Self {
            config,
            curves: curves_map,
            wireless,
            wired_devices,
            rgb_drift_enabled,
            rgb_drift_interval,
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
            daemon_tx,
        }
    }

    pub fn start(&mut self) {
        let config = self.config.clone();
        let curves = self.curves.clone();
        let wireless = self.wireless.clone();
        let wired = Arc::clone(&self.wired_devices);
        let stop_flag = Arc::clone(&self.stop_flag);
        let daemon_tx = self.daemon_tx.clone();
        let rgb_drift_enabled = self.rgb_drift_enabled;
        let rgb_drift_interval = self.rgb_drift_interval;
        let all_sensors = lianli_shared::sensors::enumerate_sensors();

        let thread = thread::spawn(move || {
            fan_control_thread(
                config,
                curves,
                wireless,
                wired,
                stop_flag,
                daemon_tx,
                &all_sensors,
                rgb_drift_enabled,
                rgb_drift_interval,
            );
        });

        self.thread = Some(thread);
    }

    pub fn stop(self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread {
            let _ = thread.join();
        }
    }
}

fn fan_control_thread(
    config: FanConfig,
    curves: HashMap<String, FanCurve>,
    wireless: Option<Arc<WirelessController>>,
    wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
    stop_flag: Arc<AtomicBool>,
    daemon_tx: Option<Sender<DaemonEvent>>,
    all_sensors: &[SensorInfo],
    rgb_drift_enabled: bool,
    rgb_drift_interval: Duration,
) {
    let update_interval = Duration::from_millis(config.update_interval_ms);
    let heartbeat_interval = Duration::from_secs(1);
    let mut last_update = Instant::now() - update_interval;
    let mut last_heartbeat = Instant::now() - heartbeat_interval;
    let mut last_drift_check = Instant::now() - rgb_drift_interval;

    // Wait briefly for wireless discovery if we have wireless
    if let Some(ref w) = wireless {
        info!("Fan control thread started, waiting for wireless discovery...");
        let discovery_start = Instant::now();
        while !stop_flag.load(Ordering::Relaxed)
            && discovery_start.elapsed() < Duration::from_secs(10)
        {
            if w.has_discovered_devices() {
                let devices = w.devices();
                info!("Wireless discovery complete: {} device(s)", devices.len());
                for dev in &devices {
                    info!("  {} — {:?}, {} fan(s)", dev, dev.fan_type, dev.fan_count);
                }
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    if !wired.is_empty() {
        let wired_names: Vec<&str> = wired.keys().map(|s| s.as_str()).collect();
        info!("Wired fan devices: {}", wired_names.join(", "));
    }

    if let Some(ref tx) = daemon_tx {
        let _ = tx.send(DaemonEvent::ResyncWirelessRgb);
    }

    if wireless
        .as_ref()
        .map_or(true, |w| !w.has_discovered_devices())
        && wired.is_empty()
    {
        warn!("No fan devices available — fan control disabled");
        return;
    }

    info!(
        "Starting fan speed control loop ({} group(s))",
        config.speeds.len()
    );

    let mut temp_ema: HashMap<SensorSource, f32> = HashMap::new();
    let mut sensor_cache: HashMap<SensorSource, ResolvedSensor> = HashMap::new();
    let mut fan_states: HashMap<usize, FanState> = HashMap::new();

    // Auto-detect CPU/GPU temp sensors for the wireless LCD clock-sync payload.
    let cpu_temp_source = picker::find_default_cpu_temp(all_sensors);
    let gpu_temp_source = picker::find_default_gpu_temp(all_sensors);

    let mut mb_sync_init: HashMap<String, HashMap<u8, bool>> = HashMap::new();
    for group in config.speeds.iter() {
        let is_mb_sync = group.speeds.iter().any(|s| s.is_mb_sync());
        if let Some(ref device_id) = group.device_id {
            if let Some((base_id, port_str)) = device_id.rsplit_once(":port") {
                if let Ok(port) = port_str.parse::<u8>() {
                    mb_sync_init
                        .entry(base_id.to_string())
                        .or_default()
                        .insert(port, is_mb_sync);
                }
            } else if let Some(dev) = wired.get(device_id) {
                for (port, _) in dev.fan_port_info() {
                    mb_sync_init
                        .entry(device_id.clone())
                        .or_default()
                        .insert(port, is_mb_sync);
                }
            }
        }
    }
    for (device_id, ports) in mb_sync_init {
        if let Some((base_id, _)) = device_id.rsplit_once(":port") {
            if let Some(dev) = wired.get(base_id) {
                if dev.supports_mb_sync() {
                    for (port, sync) in ports {
                        if let Err(err) = dev.set_mb_rpm_sync(port, sync) {
                            warn!("Failed to set MB sync for {device_id} port {port}: {err}");
                        } else if sync {
                            info!("MB RPM sync enabled for {device_id} port {port}");
                        }
                    }
                }
            }
        } else if let Some(dev) = wired.get(&device_id) {
            if dev.supports_mb_sync() {
                for (port, sync) in ports {
                    if let Err(err) = dev.set_mb_rpm_sync(port, sync) {
                        warn!("Failed to set MB sync for {device_id} port {port}: {err}");
                    } else if sync {
                        info!("MB RPM sync enabled for {device_id} port {port}");
                    }
                }
            }
        }
    }

    // Enable FgSync on the wireless dongle when any wireless group uses MB sync.
    // This makes the RX GetDev poll include fan RPM so the dongle can measure
    // motherboard PWM duty from the FG signal.
    if let Some(ref wireless) = wireless {
        let any_wireless_mb_sync = config.speeds.iter().any(|g| {
            g.device_id
                .as_ref()
                .map_or(false, |id| id.starts_with("wireless:"))
                && g.speeds.iter().any(|s| s.is_mb_sync())
        });
        wireless.set_fg_sync(any_wireless_mb_sync);
    }

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        // Broadcast master clock heartbeat (RF 0x14) once per second regardless
        // of the user-configured fan update interval. Without this packet the
        // fan firmware appears to enter an autonomous fallback that briefly
        // spikes RPM. This must be sent every second.
        if now.duration_since(last_heartbeat) >= heartbeat_interval {
            if let Some(ref w) = wireless {
                let cpu_temp = cpu_temp_source
                    .as_ref()
                    .and_then(|s| resolve_and_read(s, &mut sensor_cache, all_sensors))
                    .map(|v| v as u8)
                    .unwrap_or(0);
                let gpu_temp = gpu_temp_source
                    .as_ref()
                    .and_then(|s| resolve_and_read(s, &mut sensor_cache, all_sensors))
                    .map(|v| v as u8)
                    .unwrap_or(0);
                let snap = SensorSnapshot {
                    cpu_temp,
                    gpu_temp,
                    ..Default::default()
                };
                let payload = build_payload(&snap);
                if let Err(err) = w.send_master_clock(&payload) {
                    debug!("master clock send failed: {err}");
                }
            }
            last_heartbeat = now;
        }

        // Wireless RGB drift re-sync — runs on its own cadence (configurable)
        // and can be disabled entirely. Wireless devices only.
        if rgb_drift_enabled
            && wireless.is_some()
            && now.duration_since(last_drift_check) >= rgb_drift_interval
        {
            if let Some(ref w) = wireless {
                if w.rgb_drifted() {
                    if let Some(ref tx) = daemon_tx {
                        tx.send(DaemonEvent::ResyncWirelessRgb).ok();
                    }
                }
            }
            last_drift_check = now;
        }

        let since_last = now.duration_since(last_update);
        if since_last < update_interval {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let tick_start = Instant::now();
        last_update = tick_start;

        let mut wireless_pwm_changed = false;

        for (group_idx, group) in config.speeds.iter().enumerate() {
            let is_wireless = group
                .device_id
                .as_ref()
                .map(|id| id.starts_with("wireless:"))
                .unwrap_or(false);

            // Wireless AIOs are driven by AioController; skip them here.
            if is_wireless {
                if let (Some(device_id), Some(w)) = (&group.device_id, wireless.as_ref()) {
                    let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
                    if w.devices()
                        .iter()
                        .any(|d| d.mac_str() == mac_str && d.is_aio())
                    {
                        continue;
                    }
                }
            }

            // Wired MB sync: hardware handles it natively, skip
            if !is_wireless && group.speeds.iter().any(|s| s.is_mb_sync()) {
                continue;
            }

            // SLV3 hardware sync: all slots must be MB sync, send [6,6,6,6]
            if is_wireless && group.speeds.iter().all(|s| s.is_mb_sync()) {
                if let Some(ref device_id) = group.device_id {
                    if let Some(ref w) = wireless {
                        let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
                        let is_hw_sync = w
                            .devices()
                            .iter()
                            .find(|d| d.mac_str() == mac_str)
                            .map(|d| d.fan_type.supports_hw_mobo_sync())
                            .unwrap_or(false);
                        if is_hw_sync {
                            apply_wireless_by_id(&wireless, device_id, &[6, 6, 6, 6], group_idx);
                            continue;
                        }
                    }
                }
            }

            let speeds = match calculate_fan_speeds(
                &group.speeds,
                &curves,
                &mut sensor_cache,
                &mut temp_ema,
                all_sensors,
                fan_states.get(&group_idx),
                config.hysteresis_temp,
                config.hysteresis_pwm,
                &wired,
            ) {
                Ok(speeds) => speeds,
                Err(err) => {
                    warn!("Fan speed calculation failed for group {group_idx}: {err}");
                    continue;
                }
            };

            let group_temp = group.speeds.iter().find_map(|s| match s {
                FanSpeed::Curve(name) => curves
                    .get(name)
                    .and_then(|c| temp_ema.get(&c.effective_source()).copied()),
                _ => None,
            });

            let pwm_changed = fan_states
                .get(&group_idx)
                .map(|s| s.last_pwm != speeds)
                .unwrap_or(true);

            fan_states
                .entry(group_idx)
                .and_modify(|s| s.update(speeds, group_temp))
                .or_insert_with(|| FanState::new(speeds, group_temp));

            // Try to apply to the right device
            if let Some(ref device_id) = group.device_id {
                if device_id.starts_with("wireless:") {
                    apply_wireless_by_id(&wireless, device_id, &speeds, group_idx);
                } else if let Some((base_id, port_str)) = device_id.rsplit_once(":port") {
                    if let (Some(dev), Ok(port)) = (wired.get(base_id), port_str.parse::<u8>()) {
                        let stop = dev.stop_pwm();
                        let mapped = map_stop(&speeds, stop);
                        if let Err(err) = dev.set_fan_speed(port, mapped[0]) {
                            warn!("Failed to set fan speed for {device_id}: {err}");
                        }
                    } else {
                        warn!("Fan group {group_idx}: device '{device_id}' not found");
                    }
                } else if let Some(dev) = wired.get(device_id) {
                    let stop = dev.stop_pwm();
                    let mapped = map_stop(&speeds, stop);
                    if let Err(err) = dev.set_fan_speeds(&mapped) {
                        warn!("Failed to set fan speeds for {device_id}: {err}");
                    }
                    if dev.has_pump_control() {
                        if let Err(err) = dev.set_pump_speed(mapped[3]) {
                            warn!("Failed to set pump speed for {device_id}: {err}");
                        }
                    }
                } else {
                    warn!("Fan group {group_idx}: device '{device_id}' not found");
                }
            } else {
                if let Some(ref w) = wireless {
                    if let Err(err) = w.set_fan_speeds(group_idx as u8, &speeds) {
                        warn!("Failed to set fan speeds for wireless device {group_idx}: {err}");
                    }
                    if pwm_changed {
                        wireless_pwm_changed = true;
                    }
                }
            }

            thread::sleep(Duration::from_millis(5));
        }

        // Re-apply RGB immediately if wireless fan PWM changed, to prevent
        // the ~1s rainbow flicker caused by RF fan packets interrupting the
        // device's RGB stream.
        if wireless_pwm_changed {
            if let Some(ref tx) = daemon_tx {
                tx.send(DaemonEvent::ResyncWirelessRgb).ok();
            }
        }

        let tick_elapsed = tick_start.elapsed();
        if tick_elapsed >= update_interval {
            debug!(
                "fan tick took {tick_elapsed:?}, exceeding {update_interval:?} — skipping cooldown"
            );
        }
    }

    info!("Fan control thread stopped");
}

fn map_stop(speeds: &[u8; 4], stop: u8) -> [u8; 4] {
    [
        if speeds[0] == 0 { stop } else { speeds[0] },
        if speeds[1] == 0 { stop } else { speeds[1] },
        if speeds[2] == 0 { stop } else { speeds[2] },
        if speeds[3] == 0 { stop } else { speeds[3] },
    ]
}

fn apply_wireless_by_id(
    wireless: &Option<Arc<WirelessController>>,
    device_id: &str,
    speeds: &[u8; 4],
    group_idx: usize,
) {
    let Some(w) = wireless else {
        warn!("Fan group {group_idx}: wireless not available for device {device_id}");
        return;
    };
    // Extract MAC from "wireless:AA:BB:CC:DD:EE:FF"
    let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
    // Find the device by MAC and get its list_index
    let devices = w.devices();
    if let Some(dev) = devices.iter().find(|d| d.mac_str() == mac_str) {
        if let Err(err) = w.set_fan_speeds(dev.list_index, speeds) {
            warn!("Failed to set fan speeds for {device_id}: {err}");
        }
    } else {
        warn!("Fan group {group_idx}: wireless device {device_id} not discovered");
    }
}

/// EMA smoothing factor. Lower = smoother/slower response.
/// 0.3 means ~70% of the smoothed value comes from history.
const TEMP_EMA_ALPHA: f32 = 0.3;

/// Per-group state for PWM hysteresis. EMA smooths sensor noise; this
/// suppresses PWM chatter when temp oscillates around a curve breakpoint.
#[derive(Clone, Debug)]
struct FanState {
    last_pwm: [u8; 4],
    last_temp: Option<f32>,
}

impl FanState {
    fn new(pwm: [u8; 4], temp: Option<f32>) -> Self {
        Self {
            last_pwm: pwm,
            last_temp: temp,
        }
    }

    fn update(&mut self, pwm: [u8; 4], temp: Option<f32>) {
        let pwm_changed = pwm != self.last_pwm;
        self.last_pwm = pwm;
        if pwm_changed && temp.is_some() {
            self.last_temp = temp;
        }
    }
}

/// Suppress small PWM changes when temp is near the last applied point.
/// Keeps the last PWM if both PWM delta and temp delta are below threshold;
/// otherwise applies the target.
fn apply_hysteresis(
    target_pwm: u8,
    current_temp: f32,
    fan_idx: usize,
    state: &FanState,
    hysteresis_temp: f32,
    hysteresis_pwm: u8,
) -> u8 {
    let last_pwm = state.last_pwm[fan_idx];
    let pwm_delta = target_pwm.abs_diff(last_pwm);
    let temp_delta = state
        .last_temp
        .map(|prev| (current_temp - prev).abs())
        .unwrap_or(f32::INFINITY);

    if pwm_delta < hysteresis_pwm && temp_delta < hysteresis_temp {
        last_pwm
    } else {
        target_pwm
    }
}

fn calculate_fan_speeds(
    fan_speeds: &[FanSpeed; 4],
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    temp_ema: &mut HashMap<SensorSource, f32>,
    all_sensors: &[SensorInfo],
    prev_state: Option<&FanState>,
    hysteresis_temp: f32,
    hysteresis_pwm: u8,
    wired: &HashMap<String, Box<dyn FanDevice>>,
) -> Result<[u8; 4]> {
    let mut pwm_values = [0u8; 4];

    for (i, fan_speed) in fan_speeds.iter().enumerate() {
        pwm_values[i] = match fan_speed {
            FanSpeed::Constant(value) => *value,
            _ if fan_speed.is_mb_sync() => {
                if let Some(source) = fan_speed.mb_sync_source() {
                    lianli_shared::sensors::read_pwm_header(source).unwrap_or(0)
                } else {
                    0
                }
            }
            FanSpeed::Curve(curve_name) => {
                let curve = curves
                    .get(curve_name)
                    .ok_or_else(|| anyhow::anyhow!("Curve '{curve_name}' not found"))?;

                let source = curve.effective_source();
                let temp =
                    smoothed_temperature(&source, sensor_cache, temp_ema, all_sensors, wired)?;
                let speed_percent = interpolate_curve(&curve.curve, temp);
                let target_pwm = (speed_percent * 2.55) as u8;

                let pwm = match prev_state {
                    Some(state) => apply_hysteresis(
                        target_pwm,
                        temp,
                        i,
                        state,
                        hysteresis_temp,
                        hysteresis_pwm,
                    ),
                    None => target_pwm,
                };

                debug!("Fan {i}: Temp {temp:.1}C, Speed {speed_percent:.0}%, PWM {pwm}");
                pwm
            }
        };
    }

    Ok(pwm_values)
}

fn smoothed_temperature(
    source: &SensorSource,
    cache: &mut HashMap<SensorSource, ResolvedSensor>,
    ema: &mut HashMap<SensorSource, f32>,
    all_sensors: &[SensorInfo],
    wired: &HashMap<String, Box<dyn FanDevice>>,
) -> Result<f32> {
    // Wired AIO coolant: poll the device directly, bypass sensor resolver.
    // Fan curves reference this via SensorSource::WirelessCoolant { device_id }
    // where device_id is the wired device's ID (e.g. "hid:serial123").
    if let SensorSource::WirelessCoolant { device_id } = source {
        if let Some(dev) = wired.get(device_id) {
            if let Some(temp) = dev.poll_coolant_temp() {
                if temp > 0.0 && temp <= 100.0 {
                    let smoothed = match ema.get(source) {
                        Some(&prev) => TEMP_EMA_ALPHA * temp + (1.0 - TEMP_EMA_ALPHA) * prev,
                        None => temp,
                    };
                    ema.insert(source.clone(), smoothed);
                    return Ok(smoothed);
                }
                debug!("Wired coolant out of range: {temp:.1}°C for {device_id}");
            }
            // Poll failed or out of range — use last EMA value (keeps fans at
            // last commanded speed rather than spiking to 0 or 100%).
            return ema
                .get(source)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Coolant telemetry unavailable for {device_id}"));
        }
    }

    let resolved = match cache.get(source) {
        Some(r) => r.clone(),
        None => {
            let sensor_info = all_sensors.iter().find(|s| s.source == *source);
            let divider = sensor_info.map_or(1, |s| s.divider);
            let r = sensors::resolve_sensor(source, divider).context("sensor not found")?;
            cache.insert(source.clone(), r.clone());
            r
        }
    };

    match sensors::read_sensor_value(&resolved) {
        Ok(temp) if temp > 0.0 && temp <= 100.0 => {
            let smoothed = match ema.get(source) {
                Some(&prev) => TEMP_EMA_ALPHA * temp + (1.0 - TEMP_EMA_ALPHA) * prev,
                None => temp,
            };
            ema.insert(source.clone(), smoothed);
        }
        Ok(temp) => {
            debug!("Ignoring out-of-range temperature {temp:.1}°C");
        }
        Err(err) => {
            debug!("Sensor read failed: {err}");
            cache.remove(source);
        }
    }

    ema.get(source)
        .copied()
        .context("no valid temperature readings yet")
}

fn resolve_and_read(
    source: &SensorSource,
    cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> Option<f32> {
    let resolved = cache.get(source).cloned().or_else(|| {
        let divider = all_sensors
            .iter()
            .find(|s| s.source == *source)
            .map_or(1, |s| s.divider);
        let r = sensors::resolve_sensor(source, divider)?;
        cache.insert(source.clone(), r.clone());
        Some(r)
    })?;
    match sensors::read_sensor_value(&resolved) {
        Ok(v) => Some(v),
        Err(_) => {
            cache.remove(source);
            None
        }
    }
}
