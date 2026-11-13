#[cfg(test)]
use super::cooling::TEMPERATURE_GRACE;
use super::cooling::{curve_sources, FirstSamples, TemperatureState};
use crate::service::suspend::ResumeWatch;
use crate::service::DaemonEvent;
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::{build_payload, SensorSnapshot, WirelessController};
use lianli_shared::fan::{interpolate_curve, FanConfig, FanCurve, FanSpeed};
use lianli_shared::sensors::{self, picker, ResolvedSensor, SensorInfo, SensorSource};
use std::collections::{HashMap, HashSet};
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
    pub fn matches_config(&self, config: &lianli_shared::config::AppConfig) -> bool {
        self.config == config.fans.clone().unwrap_or_default()
            && self.curves
                == config
                    .fan_curves
                    .iter()
                    .cloned()
                    .map(|curve| (curve.name.clone(), curve))
                    .collect()
            && self.rgb_drift_enabled == config.rgb_drift_detection_enabled
            && self.rgb_drift_interval
                == Duration::from_millis(config.rgb_drift_detection_interval_ms.max(100))
    }

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
        let inputs = FanControlInputs {
            config: self.config.clone(),
            curves: self.curves.clone(),
            wireless: self.wireless.clone(),
            wired: Arc::clone(&self.wired_devices),
            stop_flag: Arc::clone(&self.stop_flag),
            daemon_tx: self.daemon_tx.clone(),
            all_sensors: lianli_shared::sensors::enumerate_sensors(),
            rgb_drift_enabled: self.rgb_drift_enabled,
            rgb_drift_interval: self.rgb_drift_interval,
        };
        let thread = thread::spawn(move || fan_control_thread(inputs));

        self.thread = Some(thread);
    }

    pub fn stop(self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread {
            let _ = thread.join();
        }
    }
}

struct FanControlInputs {
    config: FanConfig,
    curves: HashMap<String, FanCurve>,
    wireless: Option<Arc<WirelessController>>,
    wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
    stop_flag: Arc<AtomicBool>,
    daemon_tx: Option<Sender<DaemonEvent>>,
    all_sensors: Vec<SensorInfo>,
    rgb_drift_enabled: bool,
    rgb_drift_interval: Duration,
}

#[derive(Default)]
struct FanSensors {
    resolved: HashMap<SensorSource, ResolvedSensor>,
    commands: sensors::CommandSampler,
    missing_curves: HashMap<String, TemperatureState>,
}

fn fan_control_thread(inputs: FanControlInputs) {
    let FanControlInputs {
        config,
        curves,
        wireless,
        wired,
        stop_flag,
        daemon_tx,
        all_sensors,
        rgb_drift_enabled,
        rgb_drift_interval,
    } = inputs;
    let all_sensors = all_sensors.as_slice();
    let update_interval = Duration::from_millis(config.update_interval_ms.clamp(100, 10_000));
    let heartbeat_interval = Duration::from_secs(1);
    let mut last_update = Instant::now() - update_interval;
    let mut last_heartbeat = Instant::now() - heartbeat_interval;
    let mut last_drift_check = Instant::now() - rgb_drift_interval;

    if !wired.is_empty() {
        let wired_names: Vec<&str> = wired.keys().map(|s| s.as_str()).collect();
        info!("Wired fan devices: {}", wired_names.join(", "));
    }

    if let Some(ref tx) = daemon_tx {
        let _ = tx.send(DaemonEvent::ResyncWirelessRgb);
    }

    if wireless.is_none() && wired.is_empty() {
        if curves.is_empty() && config.speeds.is_empty() {
            debug!("No fan devices or fan settings; fan control idle");
        } else {
            warn!("No fan devices available — fan control disabled");
        }
        return;
    }

    info!(
        "Starting fan speed control loop ({} group(s))",
        config.speeds.len()
    );

    let mut temperatures: HashMap<SensorSource, TemperatureState> = HashMap::new();
    let mut sensor_cache = FanSensors::default();
    let mut fan_states: HashMap<usize, FanState> = HashMap::new();
    let mut unavailable_wireless = HashSet::new();
    let mut failures = super::failure_log::FailureLog::default();
    let sampled_sources = || {
        curve_sources(
            config.speeds.iter().flat_map(|group| group.speeds.iter()),
            |name| curves.get(name),
        )
    };
    let mut first_samples = FirstSamples::new(sampled_sources(), Instant::now());
    let mut resume_watch = ResumeWatch::new();

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
                .is_some_and(|id| id.starts_with("wireless:"))
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
                if let Err(err) = w.queue_master_clock(&payload) {
                    debug!("master clock not queued: {err}");
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

        if resume_watch.resumed() {
            first_samples = FirstSamples::new(sampled_sources(), now);
        }
        let samples_arrived = first_samples.waiting()
            && first_samples.refresh(now, |source| {
                read_temperature(source, &mut sensor_cache, all_sensors, &wired)
            });
        let since_last = now.duration_since(last_update);
        if since_last < update_interval
            && !samples_arrived
            && !temperatures.values().any(|state| state.needs_poll(now))
        {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let tick_start = Instant::now();
        last_update = tick_start;

        let mut wireless_pwm_changed = false;

        // RF bound wired devices are driven via the wireless path
        let bound_wireless_macs: HashSet<[u8; 6]> = wireless
            .as_ref()
            .map(|w| w.devices().iter().map(|d| d.mac).collect())
            .unwrap_or_default();

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

            if group.speeds.iter().any(FanSpeed::is_off)
                || (!is_wireless && group.speeds.iter().any(|s| s.is_mb_sync()))
            {
                continue;
            }

            // SLV3 hardware sync: all slots must be MB sync, send [6,6,6,6]
            if is_wireless && group.speeds.iter().all(|s| s.is_mb_sync()) {
                if let Some(ref device_id) = group.device_id {
                    if let Some(ref w) = wireless {
                        let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
                        let hw_sync_device = w
                            .devices()
                            .iter()
                            .find(|d| d.mac_str() == mac_str)
                            .filter(|d| d.fan_type.supports_hw_mobo_sync())
                            .map(|d| d.mac);
                        if let Some(mac) = hw_sync_device {
                            failures.record(
                                device_id,
                                "set hardware PWM sync",
                                w.set_hardware_pwm_sync(&mac),
                            );
                            continue;
                        }
                    }
                }
            }

            if group.speeds.iter().any(|speed| match speed {
                FanSpeed::Curve(name) => curves
                    .get(name)
                    .is_some_and(|curve| first_samples.is_pending(&curve.effective_source())),
                _ => false,
            }) {
                continue;
            }

            let speeds = calculate_fan_speeds(
                &group.speeds,
                &curves,
                &mut sensor_cache,
                &mut temperatures,
                all_sensors,
                FanHysteresis {
                    previous: fan_states.get(&group_idx),
                    temperature: config.hysteresis_temp,
                    pwm: config.hysteresis_pwm,
                    now: tick_start,
                },
                &wired,
            );

            let group_temp = group.speeds.iter().find_map(|s| match s {
                FanSpeed::Curve(name) => curves
                    .get(name)
                    .and_then(|c| temperatures.get(&c.effective_source()))
                    .and_then(|state| state.current),
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
                    if apply_wireless_by_id(&wireless, device_id, &speeds, &mut failures) {
                        unavailable_wireless.remove(&group_idx);
                    } else if unavailable_wireless.insert(group_idx) {
                        warn!("Fan group {group_idx}: waiting for wireless device {device_id}");
                    }
                } else if let Some((base_id, port_str)) = device_id.rsplit_once(":port") {
                    if let (Some(dev), Ok(port)) = (wired.get(base_id), port_str.parse::<u8>()) {
                        if !dev.is_ready_for_control() {
                            continue;
                        }
                        if dev
                            .wireless_link_mac()
                            .is_some_and(|m| bound_wireless_macs.contains(&m))
                        {
                            debug!("Skipping RF-bound wired device {device_id}");
                            continue;
                        }
                        let stop = dev.stop_pwm();
                        let mapped = map_stop(&speeds, stop);
                        failures.record(
                            device_id,
                            "set fan speed",
                            dev.set_fan_speed(port, mapped[0]),
                        );
                    } else {
                        failures.record(device_id, "set fan speed", Err("device not found"));
                    }
                } else if let Some(dev) = wired.get(device_id) {
                    if !dev.is_ready_for_control() {
                        continue;
                    }
                    if dev
                        .wireless_link_mac()
                        .is_some_and(|m| bound_wireless_macs.contains(&m))
                    {
                        debug!("Skipping RF-bound wired device {device_id}");
                        continue;
                    }
                    let stop = dev.stop_pwm();
                    let mapped = map_stop(&speeds, stop);
                    failures.record(device_id, "set fan speeds", dev.set_fan_speeds(&mapped));
                    if dev.has_pump_control() {
                        failures.record(device_id, "set pump speed", dev.set_pump_speed(mapped[3]));
                    }
                } else {
                    failures.record(device_id, "set fan speeds", Err("device not found"));
                }
            } else {
                if let Some(ref w) = wireless {
                    failures.record(
                        &format!("wireless group {group_idx}"),
                        "set fan speeds",
                        w.set_fan_speeds(group_idx as u8, &speeds),
                    );
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

        temperatures.retain(|_, state| state.checked_at == Some(tick_start));
        sensor_cache
            .missing_curves
            .retain(|_, state| state.checked_at == Some(tick_start));
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
    failures: &mut super::failure_log::FailureLog,
) -> bool {
    let Some(w) = wireless else {
        return false;
    };
    let mac_str = device_id.strip_prefix("wireless:").unwrap_or(device_id);
    let devices = w.devices();
    if let Some(dev) = devices.iter().find(|d| d.mac_str() == mac_str) {
        failures.record(
            device_id,
            "set fan speeds",
            w.set_fan_speeds(dev.list_index, speeds),
        );
        true
    } else {
        false
    }
}

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

struct FanHysteresis<'a> {
    previous: Option<&'a FanState>,
    temperature: f32,
    pwm: u8,
    now: Instant,
}

fn calculate_fan_speeds(
    fan_speeds: &[FanSpeed; 4],
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut FanSensors,
    temperatures: &mut HashMap<SensorSource, TemperatureState>,
    all_sensors: &[SensorInfo],
    hysteresis: FanHysteresis<'_>,
    wired: &HashMap<String, Box<dyn FanDevice>>,
) -> [u8; 4] {
    let mut pwm_values = [0u8; 4];

    for (i, fan_speed) in fan_speeds.iter().enumerate() {
        pwm_values[i] = match fan_speed {
            FanSpeed::Constant(value) => *value,
            _ if fan_speed.is_mb_sync() => {
                software_sync_pwm(fan_speed, lianli_shared::sensors::read_pwm_header)
            }
            FanSpeed::Curve(curve_name) => {
                let Some(curve) = curves.get(curve_name) else {
                    super::cooling::missing_curve(
                        &mut sensor_cache.missing_curves,
                        curve_name,
                        hysteresis.now,
                    );
                    pwm_values[i] = 255;
                    continue;
                };

                let source = curve.effective_source();
                let state = temperatures.entry(source.clone()).or_default();
                if state.checked_at != Some(hysteresis.now) {
                    let previous_fallback = state.fallback;
                    let reading = read_temperature(&source, sensor_cache, all_sensors, wired);
                    state.update_reading(reading, hysteresis.now, 0.3);
                    if state.fallback && !previous_fallback {
                        warn!(
                            ?source,
                            "Temperature unavailable. Affected cooling channels use 100% speed"
                        );
                    } else if state.recovered {
                        info!(?source, "Temperature recovered. Resuming cooling curves");
                    }
                }
                let Some(temp) = state.current else {
                    pwm_values[i] = 255;
                    continue;
                };
                let speed_percent = interpolate_curve(&curve.curve, temp);
                let target_pwm = (speed_percent * 2.55) as u8;

                let pwm = match hysteresis.previous {
                    Some(previous) if !state.recovered => apply_hysteresis(
                        target_pwm,
                        temp,
                        i,
                        previous,
                        hysteresis.temperature,
                        hysteresis.pwm,
                    ),
                    _ => target_pwm,
                };

                debug!("Fan {i}: Temp {temp:.1}C, Speed {speed_percent:.0}%, PWM {pwm}");
                pwm
            }
        };
    }

    pwm_values
}

fn read_temperature(
    source: &SensorSource,
    cache: &mut FanSensors,
    all_sensors: &[SensorInfo],
    wired: &HashMap<String, Box<dyn FanDevice>>,
) -> Option<lianli_shared::sensors::SensorReading> {
    if let SensorSource::WirelessCoolant { device_id } = source {
        if let Some(dev) = wired.get(device_id) {
            return dev.poll_coolant_reading();
        }
    }

    let resolved = match cache.resolved.get(source) {
        Some(r) => r.clone(),
        None => {
            let sensor_info = all_sensors.iter().find(|s| s.source == *source);
            let divider = sensor_info.map_or(1, |s| s.divider);
            let r = sensors::resolve_sensor(source, divider)?;
            cache.resolved.insert(source.clone(), r.clone());
            r
        }
    };

    let reading = match &resolved {
        ResolvedSensor::ShellCommand(command) => cache.commands.reading(command),
        _ => sensors::read_sensor_reading(&resolved),
    };
    match reading {
        Ok(temp) => Some(temp),
        Err(err) => {
            debug!("Sensor read failed: {err}");
            cache.resolved.remove(source);
            None
        }
    }
}

fn resolve_and_read(
    source: &SensorSource,
    cache: &mut FanSensors,
    all_sensors: &[SensorInfo],
) -> Option<f32> {
    let resolved = cache.resolved.get(source).cloned().or_else(|| {
        let divider = all_sensors
            .iter()
            .find(|s| s.source == *source)
            .map_or(1, |s| s.divider);
        let r = sensors::resolve_sensor(source, divider)?;
        cache.resolved.insert(source.clone(), r.clone());
        Some(r)
    })?;
    let reading = match &resolved {
        ResolvedSensor::ShellCommand(command) => {
            cache.commands.reading(command).and_then(|reading| {
                anyhow::ensure!(
                    reading.observed_at.elapsed() < super::cooling::TEMPERATURE_GRACE,
                    "Sensor command reading expired"
                );
                Ok(reading.value)
            })
        }
        _ => sensors::read_sensor_value(&resolved),
    };
    match reading {
        Ok(v) => Some(v),
        Err(_) => {
            cache.resolved.remove(source);
            None
        }
    }
}

fn software_sync_pwm(speed: &FanSpeed, read: impl FnOnce(&str) -> Option<u8>) -> u8 {
    speed.mb_sync_source().and_then(read).unwrap_or(255)
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    #[test]
    fn missing_curve_keeps_constant_slots_and_requests_full_speed_for_affected_slots() {
        let speeds = [
            FanSpeed::Constant(64),
            FanSpeed::Curve("missing".into()),
            FanSpeed::Constant(0),
            FanSpeed::Constant(128),
        ];
        let result = calculate_fan_speeds(
            &speeds,
            &HashMap::new(),
            &mut FanSensors::default(),
            &mut HashMap::new(),
            &[],
            FanHysteresis {
                previous: None,
                temperature: 100.0,
                pwm: 255,
                now: Instant::now(),
            },
            &HashMap::new(),
        );
        assert_eq!(result, [64, 255, 0, 128]);
    }

    #[test]
    fn missing_temperatures_expire_and_recover_without_stale_smoothing() {
        let now = Instant::now();
        let mut state = TemperatureState::default();
        state.update(None, now);
        assert!(state.fallback);
        assert_eq!(state.current, None);
        state.update(Some(40.0), now);
        assert!(state.recovered);
        assert_eq!(state.current, Some(40.0));
        for reading in [
            None,
            Some(f32::NAN),
            Some(f32::INFINITY),
            Some(0.0),
            Some(101.0),
        ] {
            state.update(reading, now + Duration::from_millis(4999));
            assert_eq!(state.current, Some(40.0));
            assert!(!state.fallback);
        }
        state.update(None, now + TEMPERATURE_GRACE);
        assert!(state.fallback);
        assert_eq!(state.current, None);
        assert!(!state.needs_poll(now + Duration::from_millis(5999)));
        assert!(state.needs_poll(now + Duration::from_secs(6)));
        state.update(Some(80.0), now + Duration::from_secs(6));
        assert!(state.recovered);
        assert_eq!(state.current, Some(80.0));
        assert!(!state.needs_poll(now + Duration::from_millis(10_999)));
        assert!(state.needs_poll(now + Duration::from_secs(11)));
        state.update(Some(80.0), now + Duration::from_secs(7));
        assert!(!state.recovered);
    }

    #[test]
    fn unavailable_curve_only_drives_affected_slots_to_full_speed() {
        let source = SensorSource::Hwmon {
            name: "lianli-nonexistent-test-sensor".into(),
            label: "missing".into(),
            device_path: String::new(),
        };
        let curve = FanCurve {
            name: "test".into(),
            temp_source: Some(source.clone()),
            temp_command: String::new(),
            curve: vec![(20.0, 20.0), (100.0, 100.0)],
        };
        let curves = HashMap::from([("test".into(), curve)]);
        let mut temperatures = HashMap::new();
        let speeds = [
            FanSpeed::Constant(64),
            FanSpeed::Curve("test".into()),
            FanSpeed::Constant(0),
            FanSpeed::Curve("test".into()),
        ];
        let now = Instant::now();
        let result = calculate_fan_speeds(
            &speeds,
            &curves,
            &mut FanSensors::default(),
            &mut temperatures,
            &[],
            FanHysteresis {
                previous: None,
                temperature: 100.0,
                pwm: 255,
                now,
            },
            &HashMap::new(),
        );
        assert_eq!(result, [64, 255, 0, 255]);
        let recovered_at = now + Duration::from_secs(6);
        temperatures
            .get_mut(&source)
            .unwrap()
            .update(Some(40.0), recovered_at);
        let previous = FanState::new(result, Some(40.0));
        let recovered = calculate_fan_speeds(
            &speeds,
            &curves,
            &mut FanSensors::default(),
            &mut temperatures,
            &[],
            FanHysteresis {
                previous: Some(&previous),
                temperature: 100.0,
                pwm: 255,
                now: recovered_at,
            },
            &HashMap::new(),
        );
        assert_eq!(recovered, [64, 102, 0, 102]);
    }

    #[test]
    fn missing_software_pwm_source_runs_full_speed() {
        let missing: FanSpeed = serde_json::from_str("\"__mb_sync__\"").unwrap();
        assert_eq!(
            software_sync_pwm(&missing, |_| panic!("bare sync must not read a header")),
            255
        );
        let selected: FanSpeed = serde_json::from_str("\"__mb_sync__:test-header\"").unwrap();
        assert_eq!(software_sync_pwm(&selected, |_| Some(128)), 128);
        assert_eq!(software_sync_pwm(&selected, |_| None), 255);
        assert_eq!(software_sync_pwm(&selected, |_| Some(0)), 0);
    }

    #[test]
    fn unrelated_settings_do_not_require_restarting_fan_control() {
        let mut config = lianli_shared::config::AppConfig::default();
        let controller = FanController::new(
            config.fans.clone().unwrap_or_default(),
            config.fan_curves.clone(),
            None,
            Arc::new(HashMap::new()),
            None,
            config.rgb_drift_detection_enabled,
            Duration::from_millis(config.rgb_drift_detection_interval_ms.max(100)),
        );
        config.hardware_video = !config.hardware_video;
        config.rgb = Some(lianli_shared::rgb::RgbAppConfig::default());
        assert!(controller.matches_config(&config));
        let original = config.clone();
        config
            .fans
            .get_or_insert_with(Default::default)
            .hysteresis_pwm += 1;
        assert!(!controller.matches_config(&config));
        config = original.clone();
        config.fan_curves.push(FanCurve {
            name: "new".into(),
            temp_source: None,
            temp_command: String::new(),
            curve: vec![(20.0, 30.0)],
        });
        assert!(!controller.matches_config(&config));
        config = original;
        config.rgb_drift_detection_enabled = !config.rgb_drift_detection_enabled;
        assert!(!controller.matches_config(&config));
    }
}
