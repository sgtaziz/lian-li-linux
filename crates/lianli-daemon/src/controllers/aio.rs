use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::{
    pump_rpm_to_timer, DiscoveredDevice, WirelessController, WirelessFanType, AIO_PARAM_LEN,
};
use lianli_shared::aio::AioConfig;
use lianli_shared::config::AppConfig;
use lianli_shared::fan::{interpolate_curve, FanCurve, FanSpeed};
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::sensors::{
    enumerate_sensors, read_sensor_value, resolve_sensor, ResolvedSensor, SensorInfo, SensorSource,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const TICK: Duration = Duration::from_secs(1);

pub struct AioController {
    wireless: Arc<WirelessController>,
    wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
    state: Arc<Mutex<State>>,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct State {
    config: AppConfig,
    needs_reinit: bool,
}

/// A resolved per-channel speed target.
#[derive(Debug, Clone, Copy)]
struct ResolvedSpeed {
    /// Raw PWM duty (0-255), for protocols driven by duty.
    duty: u8,
    /// Curve output in percent (0-100), for protocols driven by an RPM
    /// envelope (wired AIO pumps).
    percent: f32,
}

impl ResolvedSpeed {
    fn constant(duty: u8) -> Self {
        Self {
            duty,
            percent: (duty as f32 / 255.0) * 100.0,
        }
    }
}

impl AioController {
    pub fn new(
        wireless: Arc<WirelessController>,
        wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
        config: AppConfig,
    ) -> Self {
        Self {
            wireless,
            wired,
            state: Arc::new(Mutex::new(State {
                config,
                needs_reinit: false,
            })),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn set_config(&self, config: AppConfig) {
        let mut state = self.state.lock();
        state.config = config;
        state.needs_reinit = true;
    }

    pub fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let wireless = Arc::clone(&self.wireless);
        let wired = Arc::clone(&self.wired);
        let state = Arc::clone(&self.state);
        let stop = Arc::clone(&self.stop_flag);
        self.thread = Some(thread::spawn(move || run(wireless, wired, state, stop)));
    }

    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for AioController {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    wireless: Arc<WirelessController>,
    wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
    state: Arc<Mutex<State>>,
    stop_flag: Arc<AtomicBool>,
) {
    let all_sensors = enumerate_sensors();
    let mut sensor_cache: HashMap<SensorSource, ResolvedSensor> = HashMap::new();
    let mut switched: HashMap<[u8; 6], ThemeSwitch> = HashMap::new();
    // Last-sent speeds for wireless slots that resolve to "no target"
    // (off / missing curve / sensor failure): hold instead of dropping to 0.
    let mut wireless_hold: HashMap<[u8; 6], [u8; 4]> = HashMap::new();

    while !stop_flag.load(Ordering::Relaxed) {
        let cfg = {
            let mut s = state.lock();
            if s.needs_reinit {
                switched.clear();
                s.needs_reinit = false;
            }
            s.config.clone()
        };
        let curves: HashMap<String, FanCurve> = cfg
            .fan_curves
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let devices: Vec<DiscoveredDevice> = wireless.devices();
        let aio_macs: HashSet<[u8; 6]> = devices.iter().map(|d| d.mac).collect();

        control_wireless(
            &wireless,
            &devices,
            &cfg,
            &curves,
            &mut switched,
            &mut wireless_hold,
            &mut sensor_cache,
            &all_sensors,
        );
        control_wired(
            &wired,
            &aio_macs,
            &cfg,
            &curves,
            &mut sensor_cache,
            &all_sensors,
        );

        let live_macs: HashSet<[u8; 6]> = devices.iter().map(|d| d.mac).collect();
        switched.retain(|m, _| live_macs.contains(m));
        wireless_hold.retain(|m, _| live_macs.contains(m));

        thread::sleep(TICK);
    }

    debug!("AioController stopped");
}

struct ThemeSwitch {
    sequence: u8,
    sent_at: Instant,
    acknowledged: bool,
}

fn control_wireless(
    wireless: &WirelessController,
    devices: &[DiscoveredDevice],
    cfg: &AppConfig,
    curves: &HashMap<String, FanCurve>,
    switched: &mut HashMap<[u8; 6], ThemeSwitch>,
    wireless_hold: &mut HashMap<[u8; 6], [u8; 4]>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) {
    for device in devices {
        if !device.is_aio() {
            continue;
        }
        let device_id = format!("wireless:{}", device.mac_str());
        let Some(aio_cfg) = cfg.aio.get(&device_id) else {
            continue;
        };

        let needs_switch = match switched.get_mut(&device.mac) {
            Some(state) => {
                if !state.acknowledged
                    && wireless.wireless_theme_acked(&device.mac, state.sequence, state.sent_at)
                {
                    state.acknowledged = true;
                    info!(
                        "AIO {}: wireless theme command acknowledged",
                        device.mac_str()
                    );
                }
                !state.acknowledged && state.sent_at.elapsed() >= Duration::from_secs(2)
            }
            None => true,
        };
        if needs_switch {
            let sent_at = Instant::now();
            match wireless.switch_to_wireless_theme(&device.mac) {
                Ok(sequence) => {
                    switched.insert(
                        device.mac,
                        ThemeSwitch {
                            sequence,
                            sent_at,
                            acknowledged: false,
                        },
                    );
                }
                Err(e) => {
                    warn!(
                        "AIO {}: switch_to_wireless_theme failed: {e:#}",
                        device.mac_str()
                    );
                }
            }
        }

        let param = build_aio_param(
            aio_cfg,
            device,
            curves,
            sensor_cache,
            all_sensors,
            wireless_hold,
        );
        if let Err(e) = wireless.set_aio_params(&device.mac, &param) {
            warn!("AIO {}: set_aio_params failed: {e:#}", device.mac_str());
        }

        let hold = wireless_hold
            .entry(device.mac)
            .or_insert([128, 128, 128, 128]);
        let mut fan_pwm = *hold;
        let mut any_target = false;
        for (i, slot) in aio_cfg.fan_speeds.iter().enumerate() {
            if (i as u8) >= device.fan_count {
                continue;
            }
            // Unresolvable slots (off / missing curve / sensor failure) hold
            // their last commanded duty instead of dropping to 0.
            if let Some(speed) = resolve_speed(slot, curves, sensor_cache, all_sensors) {
                fan_pwm[i] = speed.duty;
                any_target = true;
            }
        }
        *hold = fan_pwm;
        // All slots device-managed → withhold the RF write entirely.
        if any_target {
            if let Err(e) = wireless.set_fan_speeds_by_mac(&device.mac, &fan_pwm) {
                warn!("AIO {}: set_fan_speeds failed: {e:#}", device.mac_str());
            }
        }
    }
}

fn control_wired(
    wired: &HashMap<String, Box<dyn FanDevice>>,
    aio_macs: &HashSet<[u8; 6]>,
    cfg: &AppConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) {
    for (base_id, dev) in wired.iter() {
        if !dev.has_pump_control() {
            continue;
        }
        // Wired hubs bridged to a wireless AIO are driven over RF instead.
        if dev
            .wireless_link_mac()
            .is_some_and(|m| aio_macs.contains(&m))
        {
            continue;
        }
        // Gate on device init (HydroShift: 10s settle + fw + handshake + 2s).
        if !dev.is_ready_for_control() {
            continue;
        }
        let Some(aio_cfg) = cfg.aio.get(base_id) else {
            // No config → device-managed, no writes.
            continue;
        };

        apply_wired_fans(
            base_id,
            dev.as_ref(),
            aio_cfg,
            curves,
            sensor_cache,
            all_sensors,
        );
        apply_wired_pump(
            base_id,
            dev.as_ref(),
            aio_cfg,
            curves,
            sensor_cache,
            all_sensors,
        );
    }
}

/// Explain, once per minute per device, why a fan tick wrote nothing.
fn warn_unresolvable(base_id: &str, aio_cfg: &AioConfig, curves: &HashMap<String, FanCurve>) {
    use std::sync::Mutex as StdMutex;
    use std::time::Instant;
    // Per device: one AIO going quiet must not mute the others.
    static LAST: StdMutex<Option<HashMap<String, Instant>>> = StdMutex::new(None);
    let mut guard = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let seen = guard.get_or_insert_with(HashMap::new);
    if seen
        .get(base_id)
        .map(|t| t.elapsed() < Duration::from_secs(60))
        .unwrap_or(false)
    {
        return;
    }
    seen.insert(base_id.to_string(), Instant::now());
    drop(guard);

    let names: Vec<&str> = aio_cfg
        .fan_speeds
        .iter()
        .filter_map(|s| match s {
            FanSpeed::Curve(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| {
            curves
                .get(*n)
                .map(|c| c.temp_source.is_none())
                .unwrap_or(true)
        })
        .collect();
    if missing.is_empty() {
        warn!("AIO {base_id}: no fan slot resolved to a duty, nothing written");
    } else {
        warn!(
            "AIO {base_id}: no fan slot resolved to a duty, nothing written — \
             curve(s) {:?} have no temp source and the default \
             /sys/class/thermal/thermal_zone0/temp is unreadable here; \
             pick a sensor for them or the fans will never be driven",
            missing
        );
    }
}

fn apply_wired_fans(
    base_id: &str,
    dev: &dyn FanDevice,
    aio_cfg: &AioConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) {
    let mut duties = [0u8; 4];
    let mut any = false;
    for (i, slot) in aio_cfg.fan_speeds.iter().enumerate() {
        if let Some(speed) = resolve_speed(slot, curves, sensor_cache, all_sensors) {
            duties[i] = speed.duty;
            any = true;
        }
    }
    if !any {
        // Intentional no-op configurations, where every slot is off or
        // motherboard synchronized, resolve nothing by design and must not
        // produce a warning. Only configurations that wanted a duty but
        // could not resolve one are worth explaining.
        let all_intentional = aio_cfg
            .fan_speeds
            .iter()
            .all(|s| s.is_off() || s.is_mb_sync());
        if !all_intentional {
            // FIX: this used to return silently. A curve whose sensor cannot be
            // resolved yields no duty, so nothing is written and the fans simply
            // stay where they were — no error, no log, nothing to debug against.
            // It is easy to hit: a curve with no explicit temp_source falls back to
            // reading /sys/class/thermal/thermal_zone0/temp, which does not exist
            // on plenty of systems (AMD desktops among them), so every curve built
            // without picking a sensor is a silent no-op.
            warn_unresolvable(base_id, aio_cfg, curves);
        }
        return;
    }
    // First configured slot drives the single PWM channel
    // (SetFanPWM [0, pwm] — no per-fan addressing in this family).
    if let Err(e) = dev.set_fan_speeds(&duties) {
        warn!("AIO {base_id}: set_fan_speeds failed: {e:#}");
    }
}

fn apply_wired_pump(
    base_id: &str,
    dev: &dyn FanDevice,
    aio_cfg: &AioConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) {
    let pump = &aio_cfg.pump_target_rpm;
    if pump.is_off() {
        return;
    }
    if pump.is_mb_sync() {
        // Vendor parity: L-Connect keeps sending the curve-derived PWM byte
        // alongside source=1 every second in motherboard-sync mode.
        let percent = match pump {
            FanSpeed::Curve(name) => curves
                .get(name)
                .and_then(|c| read_curve_percent(c, sensor_cache, all_sensors)),
            _ => None,
        };
        // Unresolvable MB-sync source: fall back to the device floor, which
        // set_pump_speed_source clamps anyway.
        let duty = percent.map(|p| (p * 2.55) as u8).unwrap_or(0);
        if let Err(e) = dev.set_pump_speed_source(1, duty) {
            warn!("AIO {base_id}: set_pump_speed_source failed: {e:#}");
        }
        return;
    }
    match pump {
        FanSpeed::Constant(b) => {
            if let Err(e) = dev.set_pump_speed(*b) {
                warn!("AIO {base_id}: set_pump_speed failed: {e:#}");
            }
        }
        FanSpeed::Curve(_) => {
            if let Some(speed) = resolve_speed(pump, curves, sensor_cache, all_sensors) {
                // Vendor-faithful chain: curve % → RPM in variant envelope →
                // RPM→PWM table → write. Implemented driver-side where the
                // envelope lives.
                if let Err(e) = dev.set_pump_curve_percent(0, speed.percent) {
                    warn!("AIO {base_id}: set_pump_curve_percent failed: {e:#}");
                }
            }
        }
    }
}

/// Resolve a [`FanSpeed`] to a concrete target.
///
/// Returns `None` (meaning: do not write) for the reserved "off" key,
/// MB-sync entries (handled separately for pumps, unsupported for fans),
/// missing curves, and unreadable sensors.
fn resolve_speed(
    speed: &FanSpeed,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> Option<ResolvedSpeed> {
    if speed.is_off() || speed.is_mb_sync() {
        return None;
    }
    match speed {
        FanSpeed::Constant(b) => Some(ResolvedSpeed::constant(*b)),
        FanSpeed::Curve(name) => {
            let curve = curves.get(name)?;
            let percent = read_curve_percent(curve, sensor_cache, all_sensors)?;
            Some(ResolvedSpeed {
                duty: (percent * 2.55) as u8,
                percent,
            })
        }
    }
}

fn read_curve_percent(
    curve: &FanCurve,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> Option<f32> {
    let source = curve.effective_source();
    let temp = resolve_and_read(&source, sensor_cache, all_sensors)?;
    Some(interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0))
}

fn build_aio_param(
    cfg: &AioConfig,
    device: &DiscoveredDevice,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
    wireless_hold: &mut HashMap<[u8; 6], [u8; 4]>,
) -> [u8; AIO_PARAM_LEN] {
    let mut p = [0u8; AIO_PARAM_LEN];

    let (cpu_temp, cpu_temp_ok) = read_optional(&cfg.cpu_temp_source, sensor_cache, all_sensors);
    let (cpu_load, cpu_load_ok) = read_optional(&cfg.cpu_load_source, sensor_cache, all_sensors);
    let (gpu_temp, gpu_temp_ok) = read_optional(&cfg.gpu_temp_source, sensor_cache, all_sensors);
    let (gpu_load, gpu_load_ok) = read_optional(&cfg.gpu_load_source, sensor_cache, all_sensors);

    p[0] = cpu_temp;
    p[1] = cpu_load;
    p[2] = gpu_temp;
    p[3] = gpu_load;
    p[6] = cfg.loop_interval;
    p[7] = 1;
    p[8] = cpu_temp_ok as u8;
    p[9] = cpu_load_ok as u8;
    p[10] = gpu_temp_ok as u8;
    p[11] = gpu_load_ok as u8;
    write_argb(&mut p[13..17], cfg.str_color);
    write_argb(&mut p[17..21], cfg.val_color);
    write_argb(&mut p[21..25], cfg.unit_color);
    p[25] = cfg.brightness.min(100);
    p[26] = 1;
    p[27] = cfg.theme_index.min(12);

    // Pump target. Unresolvable/off → hold the last commanded duty for the
    // timer translation rather than forcing a mid-RPM default.
    let hold_duty = wireless_hold.get(&device.mac).map(|h| h[3]).unwrap_or(128);
    let rpm = match resolve_pump_rpm(
        &cfg.pump_target_rpm,
        device.fan_type,
        curves,
        sensor_cache,
        all_sensors,
    ) {
        Some(rpm) => rpm,
        None => {
            let pct = (hold_duty as f32 / 255.0) * 100.0;
            rpm_from_percent(pct, device.fan_type).unwrap_or(0)
        }
    };
    let timer = pump_rpm_to_timer(rpm, device.fan_type).unwrap_or(0);
    p[28] = (timer >> 8) as u8;
    p[29] = (timer & 0xFF) as u8;
    p[30] = cfg.rotation.min(3);
    p
}

fn read_optional(
    source: &Option<SensorSourceConfig>,
    cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> (u8, bool) {
    let Some(cfg) = source else {
        return (0, false);
    };
    let src = cfg.to_sensor_source();
    match resolve_and_read(&src, cache, all_sensors) {
        Some(v) => (v.clamp(0.0, 99.0) as u8, true),
        None => (0, false),
    }
}

fn resolve_and_read(
    source: &SensorSource,
    cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> Option<f32> {
    let resolved = if let Some(r) = cache.get(source) {
        r.clone()
    } else {
        let divider = all_sensors
            .iter()
            .find(|s| s.source == *source)
            .map_or(1, |s| s.divider);
        let r = resolve_sensor(source, divider)?;
        cache.insert(source.clone(), r.clone());
        r
    };
    match read_sensor_value(&resolved) {
        Ok(v) => Some(v),
        Err(_) => {
            cache.remove(source);
            None
        }
    }
}

/// RPM within the variant's envelope for a 0-100% curve output.
fn rpm_from_percent(percent: f32, variant: WirelessFanType) -> Option<u32> {
    let (min_rpm, max_rpm) = variant.pump_rpm_range()?;
    let span = (max_rpm - min_rpm) as f32;
    Some((min_rpm as f32 + (percent / 100.0) * span).round() as u32)
}

fn resolve_pump_rpm(
    speed: &FanSpeed,
    variant: WirelessFanType,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> Option<u32> {
    let (min_rpm, max_rpm) = variant.pump_rpm_range()?;
    let pct = match speed {
        FanSpeed::Constant(b) => (*b as f32 / 255.0) * 100.0,
        FanSpeed::Curve(name) => {
            let curve = curves.get(name)?;
            let source = curve.effective_source();
            match resolve_and_read(&source, sensor_cache, all_sensors) {
                Some(temp) => interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0),
                None => return None,
            }
        }
    };
    let span = (max_rpm - min_rpm) as f32;
    Some((min_rpm as f32 + (pct / 100.0) * span).round() as u32)
}

fn write_argb(dst: &mut [u8], rgba: [u8; 4]) {
    dst[0] = rgba[3];
    dst[1] = rgba[0];
    dst[2] = rgba[1];
    dst[3] = rgba[2];
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_devices::wireless::WirelessFanType;

    fn fresh_cache() -> (HashMap<SensorSource, ResolvedSensor>, Vec<SensorInfo>) {
        (HashMap::new(), Vec::new())
    }

    #[test]
    fn resolve_speed_off_returns_none() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        assert!(resolve_speed(
            &FanSpeed::Curve("off".into()),
            &curves,
            &mut cache,
            &sensors
        )
        .is_none());
        assert!(resolve_speed(
            &FanSpeed::Curve("__mb_sync__".into()),
            &curves,
            &mut cache,
            &sensors
        )
        .is_none());
    }

    #[test]
    fn resolve_speed_constant() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        let s = resolve_speed(&FanSpeed::Constant(128), &curves, &mut cache, &sensors).unwrap();
        assert_eq!(s.duty, 128);
        // 128/255 ≈ 50.2%
        assert!((s.percent - 50.2).abs() < 0.05, "got {}", s.percent);
    }

    #[test]
    fn resolve_speed_missing_curve_returns_none() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        assert!(resolve_speed(
            &FanSpeed::Curve("nope".into()),
            &curves,
            &mut cache,
            &sensors
        )
        .is_none());
    }

    #[test]
    fn resolve_speed_curve_unreadable_sensor_returns_none() {
        let (mut cache, sensors) = fresh_cache();
        let curves: HashMap<String, FanCurve> = [(
            "c".into(),
            FanCurve {
                name: "c".into(),
                temp_source: None,
                temp_command: "exit 1".into(),
                curve: vec![(30.0, 30.0), (60.0, 70.0)],
            },
        )]
        .into_iter()
        .collect();
        assert!(
            resolve_speed(&FanSpeed::Curve("c".into()), &curves, &mut cache, &sensors).is_none()
        );
    }

    #[test]
    fn resolve_pump_rpm_constant_maps_linearly() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(0),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        )
        .unwrap();
        assert_eq!(rpm, 1600);
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(255),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        )
        .unwrap();
        assert_eq!(rpm, 2500);
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(128),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        )
        .unwrap();
        assert!((2048..=2054).contains(&rpm), "got {rpm}");
    }

    #[test]
    fn resolve_pump_rpm_square_uses_wider_range() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(255),
            WirelessFanType::WaterBlock2,
            &curves,
            &mut cache,
            &sensors,
        )
        .unwrap();
        assert_eq!(rpm, 3200);
    }

    #[test]
    fn resolve_pump_rpm_non_aio_returns_none() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        assert!(resolve_pump_rpm(
            &FanSpeed::Constant(128),
            WirelessFanType::Slv3Led,
            &curves,
            &mut cache,
            &sensors,
        )
        .is_none());
    }

    #[test]
    fn resolve_pump_rpm_off_returns_none() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        assert!(resolve_pump_rpm(
            &FanSpeed::Curve("off".into()),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        )
        .is_none());
    }

    #[test]
    fn read_optional_none_yields_disabled() {
        let (mut cache, sensors) = fresh_cache();
        let (val, ok) = read_optional(&None, &mut cache, &sensors);
        assert_eq!(val, 0);
        assert!(!ok);
    }
}
