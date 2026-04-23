use lianli_devices::wireless::{
    pump_rpm_to_timer, DiscoveredDevice, WirelessController, WirelessFanType, AIO_PARAM_LEN,
};
use lianli_shared::aio::AioConfig;
use lianli_shared::config::AppConfig;
use lianli_shared::fan::{FanCurve, FanSpeed};
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
const KEEPALIVE: Duration = Duration::from_secs(5);

pub struct AioController {
    wireless: Arc<WirelessController>,
    state: Arc<Mutex<State>>,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct State {
    config: AppConfig,
}

impl AioController {
    pub fn new(wireless: Arc<WirelessController>, config: AppConfig) -> Self {
        Self {
            wireless,
            state: Arc::new(Mutex::new(State { config })),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn set_config(&self, config: AppConfig) {
        self.state.lock().config = config;
    }

    pub fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let wireless = Arc::clone(&self.wireless);
        let state = Arc::clone(&self.state);
        let stop = Arc::clone(&self.stop_flag);
        self.thread = Some(thread::spawn(move || run(wireless, state, stop)));
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
    state: Arc<Mutex<State>>,
    stop_flag: Arc<AtomicBool>,
) {
    let all_sensors = enumerate_sensors();
    let mut sensor_cache: HashMap<SensorSource, ResolvedSensor> = HashMap::new();
    let mut last_sent: HashMap<[u8; 6], [u8; AIO_PARAM_LEN]> = HashMap::new();
    let mut last_sent_at: HashMap<[u8; 6], Instant> = HashMap::new();
    let mut switched: HashSet<[u8; 6]> = HashSet::new();
    let mut applied_image: HashMap<[u8; 6], std::path::PathBuf> = HashMap::new();

    while !stop_flag.load(Ordering::Relaxed) {
        let cfg = state.lock().config.clone();
        let curves: HashMap<String, FanCurve> = cfg
            .fan_curves
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let devices: Vec<DiscoveredDevice> = wireless.devices();

        for device in &devices {
            if !device.is_aio() {
                continue;
            }
            let device_id = format!("wireless:{}", device.mac_str());
            let Some(aio_cfg) = cfg.aio.get(&device_id) else {
                continue;
            };

            if !switched.contains(&device.mac) {
                match wireless.switch_to_wireless_theme(&device.mac) {
                    Ok(()) => {
                        switched.insert(device.mac);
                        info!("AIO {}: wireless theme mode engaged", device.mac_str());
                    }
                    Err(e) => {
                        warn!("AIO {}: switch_to_wireless_theme failed: {e:#}", device.mac_str());
                        continue;
                    }
                }
            }

            let param = build_aio_param(aio_cfg, device, &curves, &mut sensor_cache, &all_sensors);
            let now = Instant::now();
            let needs_send = match last_sent.get(&device.mac) {
                None => true,
                Some(prev) => {
                    *prev != param
                        || last_sent_at
                            .get(&device.mac)
                            .map(|t| now.duration_since(*t) >= KEEPALIVE)
                            .unwrap_or(true)
                }
            };

            if needs_send {
                match wireless.set_aio_params(&device.mac, &param) {
                    Ok(()) => {
                        last_sent.insert(device.mac, param);
                        last_sent_at.insert(device.mac, now);
                    }
                    Err(e) => {
                        warn!("AIO {}: set_aio_params failed: {e:#}", device.mac_str());
                    }
                }
            }

            let mut fan_pwm = [0u8; 4];
            for (i, slot) in aio_cfg.fan_speeds.iter().enumerate() {
                if (i as u8) >= device.fan_count {
                    continue;
                }
                fan_pwm[i] = match slot {
                    FanSpeed::Constant(b) => *b,
                    FanSpeed::Curve(name) => match curves.get(name) {
                        Some(curve) => {
                            let source = curve.effective_source();
                            let pct = match resolve_and_read(&source, &mut sensor_cache, &all_sensors) {
                                Some(temp) => interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0),
                                None => 0.0,
                            };
                            (pct * 2.55) as u8
                        }
                        None => 0,
                    },
                };
            }
            if let Err(e) = wireless.set_fan_speeds_by_mac(&device.mac, &fan_pwm) {
                warn!("AIO {}: set_fan_speeds failed: {e:#}", device.mac_str());
            }

            match &aio_cfg.custom_image_path {
                Some(path) if applied_image.get(&device.mac) != Some(path) => {
                    match lianli_media::image::encode_aio_image(path) {
                        Ok(bytes) => match wireless.send_aio_pic(&device.mac, &bytes) {
                            Ok(()) => {
                                info!(
                                    "AIO {}: custom image applied ({})",
                                    device.mac_str(),
                                    path.display()
                                );
                                applied_image.insert(device.mac, path.clone());
                            }
                            Err(e) => warn!(
                                "AIO {}: send_aio_pic failed: {e:#}",
                                device.mac_str()
                            ),
                        },
                        Err(e) => warn!(
                            "AIO {}: encode_aio_image({}) failed: {e:#}",
                            device.mac_str(),
                            path.display()
                        ),
                    }
                }
                None if applied_image.remove(&device.mac).is_some() => {
                    if let Err(e) = wireless.switch_to_wireless_theme(&device.mac) {
                        warn!(
                            "AIO {}: switch_to_wireless_theme (clear image) failed: {e:#}",
                            device.mac_str()
                        );
                    }
                }
                _ => {}
            }
        }

        let live_macs: HashSet<[u8; 6]> = devices.iter().map(|d| d.mac).collect();
        last_sent.retain(|m, _| live_macs.contains(m));
        last_sent_at.retain(|m, _| live_macs.contains(m));
        switched.retain(|m| live_macs.contains(m));
        applied_image.retain(|m, _| live_macs.contains(m));

        thread::sleep(TICK);
    }

    debug!("AioController stopped");
}

fn build_aio_param(
    cfg: &AioConfig,
    device: &DiscoveredDevice,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
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

    let rpm = resolve_pump_rpm(&cfg.pump_target_rpm, device.fan_type, curves, sensor_cache, all_sensors);
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

fn resolve_pump_rpm(
    speed: &FanSpeed,
    variant: WirelessFanType,
    curves: &HashMap<String, FanCurve>,
    sensor_cache: &mut HashMap<SensorSource, ResolvedSensor>,
    all_sensors: &[SensorInfo],
) -> u32 {
    let Some((min_rpm, max_rpm)) = variant.pump_rpm_range() else {
        return 0;
    };
    let pct = match speed {
        FanSpeed::Constant(b) => (*b as f32 / 255.0) * 100.0,
        FanSpeed::Curve(name) => match curves.get(name) {
            Some(curve) => {
                let source = curve.effective_source();
                match resolve_and_read(&source, sensor_cache, all_sensors) {
                    Some(temp) => interpolate_curve(&curve.curve, temp).clamp(0.0, 100.0),
                    None => 50.0,
                }
            }
            None => 50.0,
        },
    };
    let span = (max_rpm - min_rpm) as f32;
    (min_rpm as f32 + (pct / 100.0) * span).round() as u32
}

fn write_argb(dst: &mut [u8], rgba: [u8; 4]) {
    dst[0] = rgba[3];
    dst[1] = rgba[0];
    dst[2] = rgba[1];
    dst[3] = rgba[2];
}

fn interpolate_curve(curve: &[(f32, f32)], temp: f32) -> f32 {
    if curve.is_empty() {
        return 50.0;
    }
    if curve.len() == 1 {
        return curve[0].1;
    }
    let mut sorted = curve.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if temp <= sorted[0].0 {
        return sorted[0].1;
    }
    if temp >= sorted[sorted.len() - 1].0 {
        return sorted[sorted.len() - 1].1;
    }
    for i in 0..sorted.len() - 1 {
        let (t1, s1) = sorted[i];
        let (t2, s2) = sorted[i + 1];
        if temp >= t1 && temp <= t2 {
            let ratio = (temp - t1) / (t2 - t1);
            return s1 + ratio * (s2 - s1);
        }
    }
    50.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_devices::wireless::WirelessFanType;

    fn fresh_cache() -> (HashMap<SensorSource, ResolvedSensor>, Vec<SensorInfo>) {
        (HashMap::new(), Vec::new())
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
        );
        assert_eq!(rpm, 1600);
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(255),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        );
        assert_eq!(rpm, 2500);
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(128),
            WirelessFanType::WaterBlock,
            &curves,
            &mut cache,
            &sensors,
        );
        assert!(rpm >= 2048 && rpm <= 2054, "got {rpm}");
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
        );
        assert_eq!(rpm, 3200);
    }

    #[test]
    fn resolve_pump_rpm_non_aio_returns_zero() {
        let (mut cache, sensors) = fresh_cache();
        let curves = HashMap::new();
        let rpm = resolve_pump_rpm(
            &FanSpeed::Constant(128),
            WirelessFanType::Slv3Led,
            &curves,
            &mut cache,
            &sensors,
        );
        assert_eq!(rpm, 0);
    }

    #[test]
    fn read_optional_none_yields_disabled() {
        let (mut cache, sensors) = fresh_cache();
        let (val, ok) = read_optional(&None, &mut cache, &sensors);
        assert_eq!(val, 0);
        assert!(!ok);
    }
}
