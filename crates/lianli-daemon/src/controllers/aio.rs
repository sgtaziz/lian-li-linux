use super::cooling::{curve_sources, FirstSamples, TemperatureState};
use crate::service::suspend::ResumeWatch;
use lianli_devices::traits::FanDevice;
use lianli_devices::wireless::{
    pump_rpm_to_timer, DiscoveredDevice, WirelessController, WirelessFanType, AIO_PARAM_LEN,
};
use lianli_shared::aio::AioConfig;
use lianli_shared::config::AppConfig;
use lianli_shared::fan::{interpolate_curve, FanCurve, FanSpeed};
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::sensors::{
    enumerate_sensors, read_sensor_reading, resolve_sensor, ResolvedSensor, SensorInfo,
    SensorReading, SensorSource,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const TICK: Duration = Duration::from_secs(1);

struct SensorCache {
    failures: super::failure_log::FailureLog,
    commands: lianli_shared::sensors::CommandSampler,
    resolved: HashMap<SensorSource, ResolvedSensor>,
    temperatures: HashMap<SensorSource, TemperatureState>,
    missing_curves: HashMap<String, TemperatureState>,
    tick: Instant,
}

impl Default for SensorCache {
    fn default() -> Self {
        Self {
            failures: super::failure_log::FailureLog::default(),
            commands: lianli_shared::sensors::CommandSampler::default(),
            resolved: HashMap::new(),
            temperatures: HashMap::new(),
            missing_curves: HashMap::new(),
            tick: Instant::now(),
        }
    }
}

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

fn software_cooling_configured(config: &AppConfig, device_id: &str) -> bool {
    let software = |speed: &FanSpeed| !speed.is_off() && !speed.is_mb_sync();
    config
        .aio
        .get(device_id)
        .is_some_and(|aio| software(&aio.pump_target_rpm) || aio.fan_speeds.iter().any(software))
        || config.fans.as_ref().is_some_and(|fans| {
            fans.speeds.iter().any(|group| {
                group.device_id.as_deref().is_some_and(|id| {
                    id == device_id
                        || id
                            .strip_prefix(device_id)
                            .is_some_and(|suffix| suffix.starts_with(':'))
                }) && group.speeds.iter().any(software)
            })
        })
}

fn update_cooling_ownership(wired: &HashMap<String, Box<dyn FanDevice>>, config: &AppConfig) {
    for (id, device) in wired {
        device.set_software_cooling_active(software_cooling_configured(config, id));
    }
}

impl AioController {
    pub fn new(
        wireless: Arc<WirelessController>,
        wired: Arc<HashMap<String, Box<dyn FanDevice>>>,
        config: AppConfig,
    ) -> Self {
        update_cooling_ownership(&wired, &config);
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
        update_cooling_ownership(&self.wired, &config);
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
    let mut sensor_cache = SensorCache::default();
    let mut wireless_state = WirelessAioState::default();
    let present: HashSet<String> = wired
        .keys()
        .cloned()
        .chain(
            wireless
                .devices()
                .iter()
                .filter(|device| device.is_aio())
                .map(|device| format!("wireless:{}", device.mac_str())),
        )
        .collect();
    let wait_for_samples = |sensor_cache: &mut SensorCache| {
        let mut first_samples = {
            let state = state.lock();
            let cfg = &state.config;
            FirstSamples::new(
                curve_sources(
                    cfg.aio
                        .iter()
                        .filter(|(id, _)| present.contains(*id))
                        .flat_map(|(_, aio)| {
                            std::iter::once(&aio.pump_target_rpm).chain(&aio.fan_speeds)
                        }),
                    |name| cfg.fan_curves.iter().find(|curve| curve.name == name),
                ),
                Instant::now(),
            )
        };
        while first_samples.waiting() && !stop_flag.load(Ordering::Relaxed) {
            first_samples.refresh(Instant::now(), |source| {
                resolve_reading(source, sensor_cache, &all_sensors)
            });
            if first_samples.waiting() {
                thread::sleep(Duration::from_millis(100));
            }
        }
    };
    wait_for_samples(&mut sensor_cache);
    let mut resume_watch = ResumeWatch::new();

    while !stop_flag.load(Ordering::Relaxed) {
        if resume_watch.resumed() {
            wait_for_samples(&mut sensor_cache);
        }
        sensor_cache.tick = Instant::now();
        let cfg = {
            let mut s = state.lock();
            if s.needs_reinit {
                wireless_state.switched.clear();
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
            &mut wireless_state,
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

        sensor_cache
            .temperatures
            .retain(|_, state| state.checked_at == Some(sensor_cache.tick));
        sensor_cache
            .missing_curves
            .retain(|_, state| state.checked_at == Some(sensor_cache.tick));
        let live_macs: HashSet<[u8; 6]> = devices.iter().map(|d| d.mac).collect();
        wireless_state.switched.retain(|m, _| live_macs.contains(m));
        wireless_state
            .held_speeds
            .retain(|m, _| live_macs.contains(m));

        thread::sleep(TICK);
    }

    debug!("AioController stopped");
}

struct ThemeSwitch {
    sequence: u8,
    sent_at: Instant,
    acknowledged: bool,
}

#[derive(Default)]
struct WirelessAioState {
    switched: HashMap<[u8; 6], ThemeSwitch>,
    // Preserve device-managed slots when other slots change.
    held_speeds: HashMap<[u8; 6], [u8; 4]>,
}

fn control_wireless(
    wireless: &WirelessController,
    devices: &[DiscoveredDevice],
    cfg: &AppConfig,
    curves: &HashMap<String, FanCurve>,
    state: &mut WirelessAioState,
    sensor_cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) {
    let WirelessAioState {
        switched,
        held_speeds: wireless_hold,
    } = state;
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
                    sensor_cache.failures.record::<anyhow::Error>(
                        &device_id,
                        "switch wireless theme",
                        Ok(()),
                    );
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
                    sensor_cache
                        .failures
                        .record(&device_id, "switch wireless theme", Err(e));
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
        sensor_cache.failures.record(
            &device_id,
            "set AIO parameters",
            wireless.set_aio_params(&device.mac, &param),
        );

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
            sensor_cache.failures.record(
                &device_id,
                "set fan speeds",
                wireless.set_fan_speeds_by_mac(&device.mac, &fan_pwm),
            );
        }
    }
}

fn control_wired(
    wired: &HashMap<String, Box<dyn FanDevice>>,
    aio_macs: &HashSet<[u8; 6]>,
    cfg: &AppConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut SensorCache,
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

fn apply_wired_fans(
    base_id: &str,
    dev: &dyn FanDevice,
    aio_cfg: &AioConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) {
    let mut duties = [None; 4];
    for (i, slot) in aio_cfg
        .fan_speeds
        .iter()
        .take(dev.fan_slot_count() as usize)
        .enumerate()
    {
        if slot.is_mb_sync() {
            if dev.supports_mb_sync() {
                sensor_cache.failures.record(
                    base_id,
                    "set fan motherboard sync",
                    dev.set_mb_rpm_sync(i as u8, true),
                );
            }
            continue;
        }
        if let Some(speed) = resolve_speed(slot, curves, sensor_cache, all_sensors) {
            duties[i] = Some(speed.duty);
        }
    }
    if duties.iter().all(Option::is_none) {
        return;
    }
    sensor_cache.failures.record(
        base_id,
        "set fan speeds",
        dev.set_selected_fan_speeds(&duties),
    );
}

fn apply_wired_pump(
    base_id: &str,
    dev: &dyn FanDevice,
    aio_cfg: &AioConfig,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut SensorCache,
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
                .map(|c| read_curve_percent(c, sensor_cache, all_sensors)),
            _ => None,
        };
        // Unresolvable MB-sync source: fall back to the device floor, which
        // set_pump_speed_source clamps anyway.
        let duty = percent.map(|p| (p * 2.55) as u8).unwrap_or(0);
        sensor_cache.failures.record(
            base_id,
            "set pump motherboard sync",
            dev.set_pump_speed_source(1, duty),
        );
        return;
    }
    match pump {
        FanSpeed::Constant(b) => {
            sensor_cache
                .failures
                .record(base_id, "set pump speed", dev.set_pump_speed(*b));
        }
        FanSpeed::Curve(_) => {
            if let Some(speed) = resolve_speed(pump, curves, sensor_cache, all_sensors) {
                // Vendor-faithful chain: curve % → RPM in variant envelope →
                // RPM→PWM table → write. Implemented driver-side where the
                // envelope lives.
                sensor_cache.failures.record(
                    base_id,
                    "set pump speed",
                    dev.set_pump_curve_percent(0, speed.percent),
                );
            }
        }
    }
}

/// Resolve a [`FanSpeed`] to a concrete target.
///
/// Returns `None` (meaning: do not write) for the reserved "off" key,
/// and MB-sync entries (handled separately for supported channels).
fn resolve_speed(
    speed: &FanSpeed,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) -> Option<ResolvedSpeed> {
    if speed.is_off() || speed.is_mb_sync() {
        return None;
    }
    match speed {
        FanSpeed::Constant(b) => Some(ResolvedSpeed::constant(*b)),
        FanSpeed::Curve(name) => {
            let percent = match curves.get(name) {
                Some(curve) => read_curve_percent(curve, sensor_cache, all_sensors),
                None => super::cooling::missing_curve(
                    &mut sensor_cache.missing_curves,
                    name,
                    sensor_cache.tick,
                ),
            };
            Some(ResolvedSpeed {
                duty: (percent * 2.55) as u8,
                percent,
            })
        }
    }
}

fn read_curve_percent(
    curve: &FanCurve,
    sensor_cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) -> f32 {
    let source = curve.effective_source();
    if sensor_cache
        .temperatures
        .get(&source)
        .and_then(|state| state.checked_at)
        != Some(sensor_cache.tick)
    {
        let reading = resolve_reading(&source, sensor_cache, all_sensors);
        let state = sensor_cache.temperatures.entry(source.clone()).or_default();
        let previous_fallback = state.fallback;
        state.update_reading(reading, sensor_cache.tick, 1.0);
        if state.fallback && !previous_fallback {
            warn!(
                ?source,
                "Temperature unavailable. Affected AIO cooling channels use 100% speed"
            );
        } else if state.recovered {
            info!(
                ?source,
                "Temperature recovered. Resuming AIO cooling curves"
            );
        }
    }
    let state = &sensor_cache.temperatures[&source];
    state.current.map_or(100.0, |temp| {
        interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0)
    })
}

fn build_aio_param(
    cfg: &AioConfig,
    device: &DiscoveredDevice,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut SensorCache,
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

    // Device-managed pump targets retain their previous timer translation.
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
    cache: &mut SensorCache,
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
    cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) -> Option<f32> {
    resolve_reading(source, cache, all_sensors)
        .filter(|reading| reading.observed_at.elapsed() < super::cooling::TEMPERATURE_GRACE)
        .map(|reading| reading.value)
}

fn resolve_reading(
    source: &SensorSource,
    cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) -> Option<SensorReading> {
    let resolved = if let Some(r) = cache.resolved.get(source) {
        r.clone()
    } else {
        let divider = all_sensors
            .iter()
            .find(|s| s.source == *source)
            .map_or(1, |s| s.divider);
        let r = resolve_sensor(source, divider)?;
        cache.resolved.insert(source.clone(), r.clone());
        r
    };
    let reading = match &resolved {
        ResolvedSensor::ShellCommand(command) => cache.commands.reading(command),
        _ => read_sensor_reading(&resolved),
    };
    match reading {
        Ok(v) => Some(v),
        Err(_) => {
            cache.resolved.remove(source);
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
    sensor_cache: &mut SensorCache,
    all_sensors: &[SensorInfo],
) -> Option<u32> {
    if speed.is_off() || speed.is_mb_sync() {
        return None;
    }
    let (min_rpm, max_rpm) = variant.pump_rpm_range()?;
    let pct = match speed {
        FanSpeed::Constant(b) => (*b as f32 / 255.0) * 100.0,
        FanSpeed::Curve(name) => match curves.get(name) {
            Some(curve) => read_curve_percent(curve, sensor_cache, all_sensors),
            None => super::cooling::missing_curve(
                &mut sensor_cache.missing_curves,
                name,
                sensor_cache.tick,
            ),
        },
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
    #[test]
    fn cooling_ownership_includes_constants_curves_and_legacy_ports() {
        let mut config = AppConfig::default();
        assert!(!super::software_cooling_configured(&config, "h2"));
        let aio = lianli_shared::aio::AioConfig {
            pump_target_rpm: FanSpeed::Curve("__mb_sync__".into()),
            ..Default::default()
        };
        config.aio.insert("h2".into(), aio);
        assert!(!super::software_cooling_configured(&config, "h2"));
        config.aio.get_mut("h2").unwrap().fan_speeds[0] = FanSpeed::Constant(0);
        assert!(super::software_cooling_configured(&config, "h2"));
        config.aio.clear();
        config.fans = Some(
            serde_json::from_value(serde_json::json!({
                "speeds": [{"device_id": "h2:port0", "speeds": ["cpu", "off", "off", "off"]}]
            }))
            .unwrap(),
        );
        assert!(super::software_cooling_configured(&config, "h2"));
        assert!(!super::software_cooling_configured(&config, "h"));
    }
    use super::*;
    use lianli_devices::wireless::WirelessFanType;

    struct SelectedFan {
        count: u8,
        writes: Mutex<Vec<(u8, u8)>>,
        sync: Option<Mutex<Vec<(u8, bool)>>>,
    }

    impl FanDevice for SelectedFan {
        fn set_fan_speed(&self, slot: u8, duty: u8) -> anyhow::Result<()> {
            self.writes.lock().push((slot, duty));
            Ok(())
        }
        fn set_fan_speeds(&self, _: &[u8]) -> anyhow::Result<()> {
            panic!("Selective control must not zero-fill unmanaged slots")
        }
        fn read_fan_rpm(&self) -> anyhow::Result<Vec<u16>> {
            Ok(Vec::new())
        }
        fn fan_slot_count(&self) -> u8 {
            self.count
        }
        fn supports_mb_sync(&self) -> bool {
            self.sync.is_some()
        }
        fn set_mb_rpm_sync(&self, slot: u8, enabled: bool) -> anyhow::Result<()> {
            self.sync
                .as_ref()
                .expect("unsupported sync called")
                .lock()
                .push((slot, enabled));
            Ok(())
        }
    }

    #[test]
    fn wired_fallback_skips_off_sync_and_padding_slots() {
        let mut config = AioConfig {
            fan_speeds: [
                FanSpeed::Curve("off".into()),
                FanSpeed::Constant(0),
                FanSpeed::Constant(0),
                FanSpeed::Constant(0),
            ],
            ..Default::default()
        };
        let mut device = SelectedFan {
            count: 1,
            writes: Mutex::new(Vec::new()),
            sync: None,
        };
        let (mut cache, sensors) = fresh_cache();
        apply_wired_fans(
            "test",
            &device,
            &config,
            &HashMap::new(),
            &mut cache,
            &sensors,
        );
        assert!(device.writes.lock().is_empty());
        device.count = 3;
        config.fan_speeds[1] = FanSpeed::Curve("missing".into());
        config.fan_speeds[2] = FanSpeed::Curve("__mb_sync__".into());
        apply_wired_fans(
            "test",
            &device,
            &config,
            &HashMap::new(),
            &mut cache,
            &sensors,
        );
        assert_eq!(*device.writes.lock(), vec![(1, 255)]);
        assert_eq!(
            resolve_pump_rpm(
                &FanSpeed::Curve("missing".into()),
                WirelessFanType::WaterBlock2,
                &HashMap::new(),
                &mut cache,
                &sensors
            ),
            Some(3200)
        );
        assert!(resolve_pump_rpm(
            &FanSpeed::Curve("__mb_sync__".into()),
            WirelessFanType::WaterBlock2,
            &HashMap::new(),
            &mut cache,
            &sensors
        )
        .is_none());
    }

    #[test]
    fn supported_fan_sync_is_dispatched_instead_of_silently_skipped() {
        let device = SelectedFan {
            count: 1,
            writes: Mutex::new(Vec::new()),
            sync: Some(Mutex::new(Vec::new())),
        };
        let config = AioConfig {
            fan_speeds: std::array::from_fn(|_| FanSpeed::Curve("__mb_sync__".into())),
            ..Default::default()
        };
        let (mut cache, sensors) = fresh_cache();
        apply_wired_fans(
            "test",
            &device,
            &config,
            &HashMap::new(),
            &mut cache,
            &sensors,
        );
        assert!(device.writes.lock().is_empty());
        assert_eq!(*device.sync.as_ref().unwrap().lock(), vec![(0, true)]);
    }

    fn fresh_cache() -> (SensorCache, Vec<SensorInfo>) {
        (SensorCache::default(), Vec::new())
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
    fn aio_sensor_loss_expires_for_fans_and_both_pump_variants_then_recovers() {
        let source = SensorSource::Hwmon {
            name: "lianli-test-unavailable".into(),
            label: "temperature".into(),
            device_path: String::new(),
        };
        let curve = FanCurve {
            name: "test".into(),
            temp_source: Some(source.clone()),
            temp_command: String::new(),
            curve: vec![(20.0, 20.0), (100.0, 100.0)],
        };
        let curves = HashMap::from([("test".into(), curve)]);
        let speed = FanSpeed::Curve("test".into());
        let mut cache = SensorCache::default();
        let now = cache.tick;
        cache
            .temperatures
            .entry(source.clone())
            .or_default()
            .update_smoothed(Some(40.0), now, 1.0);
        assert_eq!(
            resolve_speed(&speed, &curves, &mut cache, &[])
                .unwrap()
                .duty,
            102
        );
        cache.tick = now + Duration::from_millis(4999);
        assert_eq!(
            resolve_speed(&speed, &curves, &mut cache, &[])
                .unwrap()
                .duty,
            102
        );
        cache.tick = now + Duration::from_secs(5);
        assert_eq!(
            resolve_speed(&speed, &curves, &mut cache, &[])
                .unwrap()
                .duty,
            255
        );
        for (variant, maximum) in [
            (WirelessFanType::WaterBlock, 2500),
            (WirelessFanType::WaterBlock2, 3200),
        ] {
            assert_eq!(
                resolve_pump_rpm(&speed, variant, &curves, &mut cache, &[]),
                Some(maximum)
            );
        }
        cache.tick = now + Duration::from_secs(6);
        cache
            .temperatures
            .get_mut(&source)
            .unwrap()
            .update_smoothed(Some(80.0), cache.tick, 1.0);
        assert_eq!(
            resolve_speed(&speed, &curves, &mut cache, &[])
                .unwrap()
                .duty,
            204
        );
        assert_eq!(
            resolve_pump_rpm(
                &speed,
                WirelessFanType::WaterBlock,
                &curves,
                &mut cache,
                &[]
            ),
            Some(2320)
        );
    }

    #[test]
    fn resolve_speed_missing_curve_runs_full_speed() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        assert_eq!(
            resolve_speed(
                &FanSpeed::Curve("nope".into()),
                &curves,
                &mut cache,
                &sensors
            )
            .unwrap()
            .duty,
            255
        );
    }

    #[test]
    fn resolve_speed_curve_unreadable_sensor_runs_full_speed() {
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
        let speed =
            resolve_speed(&FanSpeed::Curve("c".into()), &curves, &mut cache, &sensors).unwrap();
        assert_eq!(speed.duty, 255);
        assert_eq!(speed.percent, 100.0);
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
