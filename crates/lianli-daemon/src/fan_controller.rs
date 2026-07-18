use crate::service::DaemonEvent;
use anyhow::{Context, Result};
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::WirelessController;
use lianli_shared::fan::{FanConfig, FanCurve, FanSpeed};
use lianli_shared::sensors::{self, ResolvedSensor, SensorInfo, SensorSource};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const COOLANT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const COOLANT_POLL_MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct CoolantPollState {
    failures: u32,
    next_poll: Instant,
}

impl CoolantPollState {
    fn ready(now: Instant) -> Self {
        Self {
            failures: 0,
            next_poll: now,
        }
    }

    fn success(&mut self, now: Instant) {
        self.failures = 0;
        self.next_poll = now + COOLANT_POLL_INTERVAL;
    }

    fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let shift = self.failures.saturating_sub(1).min(5);
        let delay = COOLANT_POLL_INTERVAL * (1_u32 << shift);
        self.next_poll = now + delay.min(COOLANT_POLL_MAX_BACKOFF);
    }
}

pub struct FanController {
    config: FanConfig,
    curves: HashMap<String, FanCurve>,
    wireless: Option<Arc<WirelessController>>,
    wired_devices: Arc<HashMap<String, Box<dyn FanDevice>>>,
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

    info!(
        "Starting fan speed control loop ({} group(s))",
        config.speeds.len()
    );

    let mut temp_ema: HashMap<SensorSource, f32> = HashMap::new();
    let mut sensor_cache: HashMap<SensorSource, ResolvedSensor> = HashMap::new();
    let mut fan_states: HashMap<usize, FanState> = HashMap::new();
    let coolant_device_ids = coolant_devices_for_config(&config, &curves);
    let mut coolant_poll_states: HashMap<String, CoolantPollState> = coolant_device_ids
        .iter()
        .map(|id| (id.clone(), CoolantPollState::ready(Instant::now())))
        .collect();

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
                if w.rgb_drifted() {
                    if let Some(ref tx) = daemon_tx {
                        tx.send(DaemonEvent::ResyncWirelessRgb).ok();
                    }
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

        // Refresh wired AIO coolant telemetry before resolving curve sensors.
        // This also creates the runtime sensor on the first tick after boot,
        // without depending on the separate IPC telemetry polling schedule.
        for device_id in &coolant_device_ids {
            let Some(dev) = wired.get(device_id) else {
                continue;
            };
            let state = coolant_poll_states
                .entry(device_id.clone())
                .or_insert_with(|| CoolantPollState::ready(tick_start));
            if tick_start < state.next_poll {
                continue;
            }

            match dev.poll_coolant_temp() {
                Ok(Some(temp)) if temp.is_finite() && (0.0..=100.0).contains(&temp) => {
                    match sensors::write_coolant_temp(device_id, temp) {
                        Ok(()) => state.success(tick_start),
                        Err(err) => {
                            state.failure(tick_start);
                            warn!("Failed to publish coolant temperature for {device_id}: {err:#}");
                        }
                    }
                }
                Ok(Some(temp)) => {
                    state.failure(tick_start);
                    warn!("Ignoring invalid coolant temperature {temp:?} for {device_id}");
                }
                Ok(None) => {
                    state.failure(tick_start);
                    debug!("Device {device_id} does not expose coolant telemetry");
                }
                Err(err) => {
                    state.failure(tick_start);
                    warn!(
                        "Failed to poll coolant temperature for {device_id}; retry {} in {:?}: {err:#}",
                        state.failures,
                        state.next_poll.saturating_duration_since(tick_start)
                    );
                }
            }
        }

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
                }
            }

            thread::sleep(Duration::from_millis(5));
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

fn coolant_devices_for_config(
    config: &FanConfig,
    curves: &HashMap<String, FanCurve>,
) -> HashSet<String> {
    config
        .speeds
        .iter()
        .flat_map(|group| group.speeds.iter())
        .filter_map(|speed| match speed {
            FanSpeed::Curve(name) => curves.get(name),
            _ => None,
        })
        .filter_map(|curve| match curve.effective_source() {
            SensorSource::WirelessCoolant { device_id } => Some(device_id),
            _ => None,
        })
        .collect()
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
                let temp = match smoothed_temperature(&source, sensor_cache, temp_ema, all_sensors)
                {
                    Ok(temp) => temp,
                    Err(err) if matches!(source, SensorSource::WirelessCoolant { .. }) => {
                        warn!(
                            "Coolant sensor unavailable for fan {i}; applying full-speed fail-safe: {err:#}"
                        );
                        pwm_values[i] = u8::MAX;
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                let speed_percent = interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0);
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
) -> Result<f32> {
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
            if matches!(source, SensorSource::WirelessCoolant { .. }) {
                ema.remove(source);
            }
        }
        Err(err) => {
            debug!("Sensor read failed: {err}");
            cache.remove(source);
            if matches!(source, SensorSource::WirelessCoolant { .. }) {
                ema.remove(source);
            }
        }
    }

    ema.get(source)
        .copied()
        .context("no valid temperature readings yet")
}

fn interpolate_curve(curve: &[(f32, f32)], temp: f32) -> f32 {
    if curve.is_empty() {
        return 50.0;
    }

    if curve.len() == 1 {
        return curve[0].1;
    }

    let mut sorted_curve = curve.to_vec();
    sorted_curve.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    if temp <= sorted_curve[0].0 {
        return sorted_curve[0].1;
    }

    if temp >= sorted_curve[sorted_curve.len() - 1].0 {
        return sorted_curve[sorted_curve.len() - 1].1;
    }

    for i in 0..sorted_curve.len() - 1 {
        let (temp1, speed1) = sorted_curve[i];
        let (temp2, speed2) = sorted_curve[i + 1];

        if temp >= temp1 && temp <= temp2 {
            let ratio = (temp - temp1) / (temp2 - temp1);
            return speed1 + ratio * (speed2 - speed1);
        }
    }

    50.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::fan::FanGroup;

    fn coolant_curve(device_id: &str) -> FanCurve {
        FanCurve {
            name: "coolant".into(),
            temp_source: Some(SensorSource::WirelessCoolant {
                device_id: device_id.into(),
            }),
            temp_command: String::new(),
            curve: vec![(25.0, 20.0), (45.0, 100.0)],
        }
    }

    #[test]
    fn only_configured_coolant_devices_are_polled() {
        let curve = coolant_curve("hid:aio");
        let curves = HashMap::from([(curve.name.clone(), curve)]);
        let config = FanConfig {
            speeds: vec![FanGroup {
                device_id: Some("hid:aio".into()),
                speeds: [
                    FanSpeed::Curve("coolant".into()),
                    FanSpeed::Constant(64),
                    FanSpeed::Constant(64),
                    FanSpeed::Constant(128),
                ],
            }],
            ..FanConfig::default()
        };

        assert_eq!(
            coolant_devices_for_config(&config, &curves),
            HashSet::from(["hid:aio".to_string()])
        );
        assert!(coolant_devices_for_config(&FanConfig::default(), &curves).is_empty());
    }

    #[test]
    fn coolant_poll_failures_back_off_and_recovery_resets_delay() {
        let now = Instant::now();
        let mut state = CoolantPollState::ready(now);
        state.failure(now);
        assert_eq!(state.failures, 1);
        assert_eq!(state.next_poll.duration_since(now), Duration::from_secs(1));

        let second = state.next_poll;
        state.failure(second);
        assert_eq!(
            state.next_poll.duration_since(second),
            Duration::from_secs(2)
        );

        state.success(state.next_poll);
        assert_eq!(state.failures, 0);
        assert_eq!(
            state
                .next_poll
                .duration_since(second + Duration::from_secs(2)),
            COOLANT_POLL_INTERVAL
        );
    }

    #[test]
    fn missing_coolant_sensor_uses_full_speed_fail_safe() {
        let source = SensorSource::WirelessCoolant {
            device_id: format!("missing-test-device-{}", std::process::id()),
        };
        let curve = FanCurve {
            name: "coolant".into(),
            temp_source: Some(source),
            temp_command: String::new(),
            curve: vec![(25.0, 20.0), (45.0, 100.0)],
        };
        let curves = HashMap::from([(curve.name.clone(), curve)]);
        let mut cache = HashMap::new();
        let mut ema = HashMap::new();
        let speeds = calculate_fan_speeds(
            &[
                FanSpeed::Curve("coolant".into()),
                FanSpeed::Constant(64),
                FanSpeed::Constant(64),
                FanSpeed::Constant(128),
            ],
            &curves,
            &mut cache,
            &mut ema,
            &[],
            None,
            1.0,
            5,
        )
        .unwrap();
        assert_eq!(speeds, [u8::MAX, 64, 64, 128]);
    }

    #[test]
    fn curve_percent_is_clamped_before_pwm_conversion() {
        let source = SensorSource::Command {
            cmd: "unused".into(),
        };
        let curve = FanCurve {
            name: "bad-range".into(),
            temp_source: Some(source.clone()),
            temp_command: "unused".into(),
            curve: vec![(20.0, -50.0), (40.0, 150.0)],
        };
        let curves = HashMap::from([(curve.name.clone(), curve)]);
        let mut cache = HashMap::from([(source, ResolvedSensor::Constant(40.0))]);
        let mut ema = HashMap::new();
        let speeds = calculate_fan_speeds(
            &[
                FanSpeed::Curve("bad-range".into()),
                FanSpeed::Constant(0),
                FanSpeed::Constant(0),
                FanSpeed::Constant(0),
            ],
            &curves,
            &mut cache,
            &mut ema,
            &[],
            None,
            1.0,
            5,
        )
        .unwrap();
        assert_eq!(speeds[0], u8::MAX);
    }
}
