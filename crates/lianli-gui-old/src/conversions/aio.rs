use super::device::family_display_name;
use super::fan::fan_speed_to_slot;
use lianli_shared::aio::AioConfig;
use lianli_shared::config::AppConfig;
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::{DeviceInfo, TelemetrySnapshot};
use lianli_shared::media::SensorSourceConfig;
use slint::{ModelRc, SharedString, VecModel};

pub fn aio_sensor_options_model(
    sensors: &[lianli_shared::sensors::SensorInfo],
) -> ModelRc<SharedString> {
    let mut items: Vec<SharedString> = vec![SharedString::from("None")];
    for s in sensors {
        items.push(SharedString::from(s.get_display_name()));
    }
    ModelRc::new(VecModel::from(items))
}

pub fn aio_theme_options_model() -> ModelRc<SharedString> {
    let items: Vec<SharedString> = (0..=12)
        .map(|i| SharedString::from(format!("Theme {i}")))
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn aio_rotation_options_model() -> ModelRc<SharedString> {
    let items: Vec<SharedString> = ["0°", "90°", "180°", "270°"]
        .iter()
        .map(|s| SharedString::from(*s))
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn aio_sensor_index(
    source: &Option<SensorSourceConfig>,
    sensors: &[lianli_shared::sensors::SensorInfo],
) -> i32 {
    let Some(cfg) = source else {
        return 0;
    };
    let target = cfg.to_sensor_source();
    sensors
        .iter()
        .position(|s| s.source == target)
        .map(|i| (i as i32) + 1)
        .unwrap_or(0)
}

pub fn aios_to_model(
    config: &AppConfig,
    devices: &[DeviceInfo],
    telemetry: &TelemetrySnapshot,
    sensors: &[lianli_shared::sensors::SensorInfo],
    pwm_headers: &[lianli_shared::sensors::PwmHeader],
) -> ModelRc<crate::AioData> {
    let items: Vec<crate::AioData> = config
        .aio
        .iter()
        .map(|(device_id, cfg)| {
            aio_to_slint(device_id, cfg, devices, telemetry, sensors, pwm_headers)
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn aio_to_slint(
    device_id: &str,
    cfg: &AioConfig,
    devices: &[DeviceInfo],
    telemetry: &TelemetrySnapshot,
    sensors: &[lianli_shared::sensors::SensorInfo],
    pwm_headers: &[lianli_shared::sensors::PwmHeader],
) -> crate::AioData {
    let device = devices.iter().find(|d| d.device_id == device_id);
    let display_name = device
        .map(|d| {
            if d.name.is_empty() {
                family_display_name(d.family).to_string()
            } else {
                d.name.clone()
            }
        })
        .unwrap_or_else(|| family_display_name(DeviceFamily::WirelessAio).to_string());

    let mac = device_id
        .strip_prefix("wireless:")
        .unwrap_or("")
        .to_string();

    let (min_rpm, max_rpm) = device
        .and_then(|d| d.pump_rpm_range)
        .unwrap_or((1600, 2500));
    let fan_count = device.and_then(|d| d.fan_count).unwrap_or(0) as i32;

    let current_pump_rpm = telemetry
        .fan_rpms
        .get(device_id)
        .and_then(|v| v.last().copied())
        .unwrap_or(0) as i32;
    let coolant_temp = telemetry
        .coolant_temps
        .get(device_id)
        .map(|t| format!("Coolant: {t:.1}\u{00B0}C"))
        .unwrap_or_default();

    let pump_slot = fan_speed_to_slot(&cfg.pump_target_rpm, pwm_headers);
    let slots: Vec<crate::FanSpeedSlot> = cfg
        .fan_speeds
        .iter()
        .map(|s| fan_speed_to_slot(s, pwm_headers))
        .collect();

    crate::AioData {
        device_id: SharedString::from(device_id),
        display_name: SharedString::from(&display_name),
        mac: SharedString::from(&mac),
        min_rpm: min_rpm as i32,
        max_rpm: max_rpm as i32,
        current_pump_rpm,
        coolant_temp: SharedString::from(&coolant_temp),
        fan_count,
        pump_slot,
        slots: ModelRc::new(VecModel::from(slots)),
        cpu_temp_index: aio_sensor_index(&cfg.cpu_temp_source, sensors),
        cpu_load_index: aio_sensor_index(&cfg.cpu_load_source, sensors),
        gpu_temp_index: aio_sensor_index(&cfg.gpu_temp_source, sensors),
        gpu_load_index: aio_sensor_index(&cfg.gpu_load_source, sensors),
        str_r: cfg.str_color[0] as i32,
        str_g: cfg.str_color[1] as i32,
        str_b: cfg.str_color[2] as i32,
        str_a: cfg.str_color[3] as i32,
        val_r: cfg.val_color[0] as i32,
        val_g: cfg.val_color[1] as i32,
        val_b: cfg.val_color[2] as i32,
        val_a: cfg.val_color[3] as i32,
        unit_r: cfg.unit_color[0] as i32,
        unit_g: cfg.unit_color[1] as i32,
        unit_b: cfg.unit_color[2] as i32,
        unit_a: cfg.unit_color[3] as i32,
        brightness: cfg.brightness as i32,
        rotation: cfg.rotation as i32,
        theme_index: cfg.theme_index as i32,
        loop_interval: cfg.loop_interval as i32,
    }
}
