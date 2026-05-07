use anyhow::{Context, Result};
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::WirelessController;
use lianli_shared::fan::{FanConfig, FanCurve, FanSpeed};
use lianli_shared::sensors::{self, ResolvedSensor, SensorInfo, SensorSource};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
struct FanState {
    last_temp: Option<f32>,
    last_pwm: [u8; 4],
    last_direction: [i8; 4],
}

pub struct FanController {
    config: FanConfig,
    curves: HashMap<String, FanCurve>,
    wireless: Option<Arc<WirelessController>>,
    wired_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FanController {
    pub fn new(
        config: FanConfig,
        curves: Vec<FanCurve>,
        wireless: Option<Arc<WirelessController>>,
        wired_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
    ) -> Self {
        let curves_map: HashMap<String, FanCurve> =
            curves.into_iter().map(|c| (c.name.clone(), c)).collect();

        Self {
            config,
            curves: curves_map,
            wireless,
            wired_devices,
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn start(&mut self) {
        let config = self.config.clone();
        let curves = self.curves.clone();
        let wireless = self.wireless.clone();
        let wired = Arc::clone(&self.wired_devices);
        let stop_flag = Arc::clone(&self.stop_flag);
        let all_sensors = lianli_shared::sensors::enumerate_sensors();

        let thread = thread::spawn(move || {
            fan_control_thread(config, curves, wireless, wired, stop_flag, &all_sensors);
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
    all_sensors: &[SensorInfo],
) {
    let update_interval = Duration::from_millis(config.update_interval_ms);
    let heartbeat_interval = Duration::from_secs(1);
    let mut last_update = Instant::now() - update_interval;
    let mut last_heartbeat = Instant::now() - heartbeat_interval;

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

    if wireless
        .as_ref()
        .map_or(true, |w| !w.has_discovered_devices())
        && wired.is_empty()
    {
        warn!("No fan devices available — fan control disabled");
        return;
    }

    debug!(
        "Starting fan speed control loop ({} group(s), update_interval={}ms, hysteresis_temp={:.1}°C, hysteresis_pwm={})",
        config.speeds.len(),
        config.update_interval_ms,
        config.hysteresis_temp,
        config.hysteresis_pwm
    );

    for (idx, group) in config.speeds.iter().enumerate() {
        let device_id = group.device_id.as_deref().unwrap_or("none");
        let fan_modes: Vec<&str> = group
            .speeds
            .iter()
            .map(|s| match s {
                FanSpeed::Constant(v) => "const",
                FanSpeed::Curve(c) => c.as_str(),
            })
            .collect();
        debug!(
            "Group {}: device='{}', fans=[{}]",
            idx,
            device_id,
            fan_modes.join(", ")
        );
    }

    let mut temp_ema: HashMap<SensorSource, f32> = HashMap::new();
    let mut sensor_cache: HashMap<SensorSource, ResolvedSensor> = HashMap::new();
    let mut fan_states: HashMap<usize, FanState> = HashMap::new();

    // Initialize MB sync state for all wired groups at startup.
    for (group_idx, group) in config.speeds.iter().enumerate() {
        let is_mb_sync = group.speeds.iter().any(|s| s.is_mb_sync());
        if let Some(ref device_id) = group.device_id {
            if let Some((base_id, port_str)) = device_id.rsplit_once(":port") {
                if let (Some(dev), Ok(port)) = (wired.get(base_id), port_str.parse::<u8>()) {
                    if dev.supports_mb_sync() {
                        if let Err(err) = dev.set_mb_rpm_sync(port, is_mb_sync) {
                            warn!("Failed to set MB sync for {device_id}: {err}");
                        } else if is_mb_sync {
                            info!("MB RPM sync enabled for {device_id}");
                        }
                    }
                }
            } else if let Some(dev) = wired.get(device_id) {
                if dev.supports_mb_sync() {
                    if let Err(err) = dev.set_mb_rpm_sync(0, is_mb_sync) {
                        warn!("Failed to set MB sync for {device_id}: {err}");
                    } else if is_mb_sync {
                        info!("MB RPM sync enabled for {device_id}");
                    }
                }
            }
        }
        if is_mb_sync {
            debug!(
                "Group {group_idx} ({}): MB RPM sync mode",
                group.device_id.as_deref().unwrap_or("none")
            );
        }
    }

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        // Broadcast master clock heartbeat (RF 0x14) once per second regardless
        // of the user-configured fan update interval. Without this packet the
        // fan firmware appears to enter an autonomous fallback that briefly
        // spikes RPM. L-Connect sends this every second.
        if now.duration_since(last_heartbeat) >= heartbeat_interval {
            if let Some(ref w) = wireless {
                if let Err(err) = w.send_master_clock() {
                    debug!("master clock send failed: {err}");
                }
            }
            last_heartbeat = now;
        }

        let since_last = now.duration_since(last_update);
        if since_last < update_interval {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let tick_start = Instant::now();
        last_update = tick_start;

        debug!("=== Fan control tick start ===");

        for (group_idx, group) in config.speeds.iter().enumerate() {
            let is_wireless = group
                .device_id
                .as_ref()
                .map(|id| id.starts_with("wireless:"))
                .unwrap_or(false);

            debug!(
                "Group {} ({}): wireless={}, speeds={:?}",
                group_idx,
                group.device_id.as_deref().unwrap_or("none"),
                is_wireless,
                group.speeds
            );

            // Wireless AIOs are driven by AioController; skip them here.
            if is_wireless {
                if let (Some(device_id), Some(w)) = (&group.device_id, wireless.as_ref()) {
                    let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
                    if w.devices()
                        .iter()
                        .any(|d| d.mac_str() == mac_str && d.is_aio())
                    {
                        debug!(
                            "Group {}: skipping wireless AIO (controlled separately)",
                            group_idx
                        );
                        continue;
                    }
                }
            }

            // Wired MB sync: hardware handles it natively, skip
            if !is_wireless && group.speeds.iter().any(|s| s.is_mb_sync()) {
                debug!(
                    "Group {}: skipping wired MB sync (hardware handles)",
                    group_idx
                );
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
                            debug!(
                                "Group {}: SLV3 hardware MB sync enabled, sending [6,6,6,6]",
                                group_idx
                            );
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
            ) {
                Ok(speeds) => speeds,
                Err(err) => {
                    warn!("Fan speed calculation failed for group {group_idx}: {err}");
                    continue;
                }
            };

            let current_temps: Vec<Option<f32>> = group
                .speeds
                .iter()
                .map(|s| {
                    if let FanSpeed::Curve(curve_name) = s {
                        curves.get(curve_name).and_then(|c| {
                            let source = c.effective_source();
                            temp_ema.get(&source).copied()
                        })
                    } else {
                        None
                    }
                })
                .collect();

            let current_directions: [i8; 4] = {
                let mut dirs = [0i8; 4];
                if let Some(state) = fan_states.get(&group_idx) {
                    for i in 0..4 {
                        let last = state.last_pwm[i];
                        if speeds[i] > last {
                            dirs[i] = 1;
                        } else if speeds[i] < last {
                            dirs[i] = -1;
                        } else {
                            dirs[i] = state.last_direction[i];
                        }
                    }
                }
                dirs
            };

            if let Some(state) = fan_states.get_mut(&group_idx) {
                state.last_pwm = speeds;
                state.last_direction = current_directions;
                if let Some(temp) = current_temps.iter().flatten().next() {
                    state.last_temp = Some(*temp);
                }
            } else {
                fan_states.insert(
                    group_idx,
                    FanState {
                        last_temp: current_temps.iter().flatten().next().copied(),
                        last_pwm: speeds,
                        last_direction: current_directions,
                    },
                );
            }

            // Try to apply to the right device
            if let Some(ref device_id) = group.device_id {
                if device_id.starts_with("wireless:") {
                    debug!(
                        "Group {}: applying wireless PWM {:?} to {}",
                        group_idx, speeds, device_id
                    );
                    apply_wireless_by_id(&wireless, device_id, &speeds, group_idx);
                } else if let Some((base_id, port_str)) = device_id.rsplit_once(":port") {
                    // Per-port wired device (e.g. "Nuvoton:port0")
                    if let (Some(dev), Ok(port)) = (wired.get(base_id), port_str.parse::<u8>()) {
                        debug!(
                            "Group {}: applying wired PWM {} to {} port {}",
                            group_idx, speeds[0], base_id, port
                        );
                        if let Err(err) = dev.set_fan_speed(port, speeds[0]) {
                            warn!("Failed to set fan speed for {device_id}: {err}");
                        }
                    } else {
                        warn!("Fan group {group_idx}: device '{device_id}' not found");
                    }
                } else if let Some(dev) = wired.get(device_id) {
                    debug!(
                        "Group {}: applying wired PWM {:?} to {}",
                        group_idx, speeds, device_id
                    );
                    if let Err(err) = dev.set_fan_speeds(&speeds) {
                        warn!("Failed to set fan speeds for {device_id}: {err}");
                    }
                    if dev.has_pump_control() {
                        debug!("Group {}: setting pump PWM to {}", group_idx, speeds[3]);
                        if let Err(err) = dev.set_pump_speed(speeds[3]) {
                            warn!("Failed to set pump speed for {device_id}: {err}");
                        }
                    }
                } else {
                    warn!("Fan group {group_idx}: device '{device_id}' not found");
                }
            } else {
                // Legacy: match by group index to wireless devices
                if let Some(ref w) = wireless {
                    debug!(
                        "Group {} (legacy): applying wireless PWM {:?} to device index {}",
                        group_idx, speeds, group_idx
                    );
                    if let Err(err) = w.set_fan_speeds(group_idx as u8, &speeds) {
                        warn!("Failed to set fan speeds for wireless device {group_idx}: {err}");
                    }
                }
            }

            thread::sleep(Duration::from_millis(5));
        }

        let tick_elapsed = tick_start.elapsed();
        debug!(
            "=== Fan control tick complete in {:?} ({} groups processed) ===",
            tick_elapsed,
            config.speeds.len()
        );
        if tick_elapsed >= update_interval {
            warn!(
                "Fan tick took {tick_elapsed:?}, exceeding {update_interval:?} — skipping cooldown"
            );
        }
    }

    info!("Fan control thread stopped");
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
        debug!(
            "Group {}: found wireless device {} (type={:?}, fans={}, list_index={})",
            group_idx, mac_str, dev.fan_type, dev.fan_count, dev.list_index
        );
        if let Err(err) = w.set_fan_speeds(dev.list_index, speeds) {
            warn!("Failed to set fan speeds for {device_id}: {err}");
        } else {
            debug!(
                "Group {}: successfully sent PWM {:?} to {}",
                group_idx, speeds, mac_str
            );
        }
    } else {
        warn!("Fan group {group_idx}: wireless device {device_id} not discovered");
        debug!(
            "Available devices: {:?}",
            devices.iter().map(|d| d.mac_str()).collect::<Vec<_>>()
        );
    }
}

/// EMA smoothing factor. Lower = smoother/slower response.
/// 0.3 means ~70% of the smoothed value comes from history.
const TEMP_EMA_ALPHA: f32 = 0.3;

fn calculate_fan_speeds(
    fan_speeds: &[FanSpeed; 4],
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    temp_ema: &mut HashMap<SensorSource, f32>,
    all_sensors: &[SensorInfo],
    fan_state: Option<&FanState>,
    hysteresis_temp: f32,
    hysteresis_pwm: u8,
) -> Result<[u8; 4]> {
    let mut pwm_values = [0u8; 4];

    for (i, fan_speed) in fan_speeds.iter().enumerate() {
        pwm_values[i] = match fan_speed {
            FanSpeed::Constant(value) => {
                debug!("Fan {}: constant PWM {}", i, value);
                *value
            }
            _ if fan_speed.is_mb_sync() => {
                let pwm = if let Some(source) = fan_speed.mb_sync_source() {
                    let val = lianli_shared::sensors::read_pwm_header(source).unwrap_or(0);
                    debug!("Fan {}: MB sync from {} -> PWM {}", i, source, val);
                    val
                } else {
                    debug!("Fan {}: MB sync (no source) -> PWM 0", i);
                    0
                };
                pwm
            }
            FanSpeed::Curve(curve_name) => {
                let curve = curves
                    .get(curve_name)
                    .ok_or_else(|| anyhow::anyhow!("Curve '{curve_name}' not found"))?;

                let source = curve.effective_source();
                let temp = smoothed_temperature(&source, sensor_cache, temp_ema, all_sensors)?;
                let speed_percent = interpolate_curve(&curve.curve, temp);
                let target_pwm = (speed_percent * 2.55) as u8;

                let final_pwm = if let Some(state) = fan_state {
                    apply_hysteresis(target_pwm, temp, i, state, hysteresis_temp, hysteresis_pwm)
                } else {
                    debug!(
                        "Fan {}: first run, no hysteresis state -> PWM {}",
                        i, target_pwm
                    );
                    target_pwm
                };

                final_pwm
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
) -> Result<f32> {
    let resolved = match cache.get(source) {
        Some(r) => r.clone(),
        None => {
            debug!("Resolving new sensor source: {:?}", source);
            let sensor_info = all_sensors.iter().find(|s| s.source == *source);
            let divider = sensor_info.map_or(1, |s| s.divider);
            let r = sensors::resolve_sensor(source, divider).context("sensor not found")?;
            cache.insert(source.clone(), r.clone());
            debug!("Sensor resolved: {:?}", r);
            r
        }
    };

    match sensors::read_sensor_value(&resolved) {
        Ok(temp) if temp > 0.0 && temp <= 100.0 => {
            let smoothed = match ema.get(source) {
                Some(&prev) => {
                    let s = TEMP_EMA_ALPHA * temp + (1.0 - TEMP_EMA_ALPHA) * prev;
                    debug!(
                        "Sensor {:?}: raw={:.1}°C, prev_ema={:.1}°C, new_ema={:.1}°C (alpha={})",
                        source, temp, prev, s, TEMP_EMA_ALPHA
                    );
                    s
                }
                None => {
                    debug!("Sensor {:?}: first reading {:.1}°C", source, temp);
                    temp
                }
            };
            ema.insert(source.clone(), smoothed);
        }
        Ok(temp) => {
            debug!(
                "Ignoring out-of-range temperature {temp:.1}°C from {:?}",
                source
            );
        }
        Err(err) => {
            debug!("Sensor read failed for {:?}: {err}", source);
            cache.remove(source);
        }
    }

    ema.get(source)
        .copied()
        .context("no valid temperature readings yet")
}

fn apply_hysteresis(
    target_pwm: u8,
    current_temp: f32,
    fan_idx: usize,
    state: &FanState,
    hysteresis_temp: f32,
    hysteresis_pwm: u8,
) -> u8 {
    let last_pwm = state.last_pwm[fan_idx];
    let pwm_diff = (target_pwm as i16 - last_pwm as i16).abs() as u8;

    let temp_diff = state
        .last_temp
        .map(|last| (current_temp - last).abs())
        .unwrap_or(f32::MAX);

    let direction = if target_pwm > last_pwm {
        1
    } else if target_pwm < last_pwm {
        -1
    } else {
        0
    };

    let last_direction = state.last_direction[fan_idx];
    let direction_changed = last_direction != 0 && direction != 0 && direction != last_direction;

    debug!(
        "Fan {}: hysteresis check — target_pwm={}, last_pwm={}, pwm_diff={}, current_temp={:.1}°C, temp_diff={:.1}°C, direction={}, last_dir={}, dir_changed={}",
        fan_idx, target_pwm, last_pwm, pwm_diff, current_temp, temp_diff, direction, last_direction, direction_changed
    );

    if pwm_diff < hysteresis_pwm && temp_diff < hysteresis_temp && !direction_changed {
        debug!(
            "Fan {}: HYSTERESIS — keeping PWM {} (target {}, thresholds: pwm_diff {} < {}, temp_diff {:.1} < {:.1}, no direction change)",
            fan_idx, last_pwm, target_pwm, pwm_diff, hysteresis_pwm, temp_diff, hysteresis_temp
        );
        last_pwm
    } else {
        if last_pwm != target_pwm {
            debug!(
                "Fan {}: PWM {} → {} (reasons: pwm_diff={} {} hysteresis_pwm={}, temp_diff={:.1} {} hysteresis_temp={:.1}, dir_changed={})",
                fan_idx,
                last_pwm,
                target_pwm,
                pwm_diff,
                if pwm_diff >= hysteresis_pwm { ">=" } else { "<" },
                hysteresis_pwm,
                temp_diff,
                if temp_diff >= hysteresis_temp { ">=" } else { "<" },
                hysteresis_temp,
                direction_changed
            );
        }
        target_pwm
    }
}

fn interpolate_curve(curve: &[(f32, f32)], temp: f32) -> f32 {
    if curve.is_empty() {
        debug!("Curve is empty, returning default 50%");
        return 50.0;
    }

    if curve.len() == 1 {
        debug!("Single-point curve, returning {}%", curve[0].1);
        return curve[0].1;
    }

    let mut sorted_curve = curve.to_vec();
    sorted_curve.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    if temp <= sorted_curve[0].0 {
        debug!(
            "Temp {:.1}°C below curve range, clamping to first point: {:.1}°C -> {}%",
            temp, sorted_curve[0].0, sorted_curve[0].1
        );
        return sorted_curve[0].1;
    }

    if temp >= sorted_curve[sorted_curve.len() - 1].0 {
        debug!(
            "Temp {:.1}°C above curve range, clamping to last point: {:.1}°C -> {}%",
            temp,
            sorted_curve[sorted_curve.len() - 1].0,
            sorted_curve[sorted_curve.len() - 1].1
        );
        return sorted_curve[sorted_curve.len() - 1].1;
    }

    for i in 0..sorted_curve.len() - 1 {
        let (temp1, speed1) = sorted_curve[i];
        let (temp2, speed2) = sorted_curve[i + 1];

        if temp >= temp1 && temp <= temp2 {
            let ratio = (temp - temp1) / (temp2 - temp1);
            let result = speed1 + ratio * (speed2 - speed1);
            debug!(
                "Curve interpolation: {:.1}°C between [{:.1}°C->{}%, {:.1}°C->{}%], ratio={:.3}, result={:.1}%",
                temp, temp1, speed1, temp2, speed2, ratio, result
            );
            return result;
        }
    }

    debug!("Curve interpolation failed, returning default 50%");
    50.0
}
