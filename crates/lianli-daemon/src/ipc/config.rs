//! Config-mutating IPC handlers: `SetLcdMedia`, `SetFanConfig`, `SetFanSpeed`,
//! `SetRgbConfig`. Each updates `state.config` in place and then calls
//! [`super::persist_and_notify`] to flush to disk + wake the daemon.

use std::sync::mpsc::Sender;

use lianli_shared::config::AppConfig;
use lianli_shared::config::LcdConfig;
use lianli_shared::fan::{FanConfig, FanGroup, FanSpeed};
use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::RgbAppConfig;
use tracing::debug;

use crate::ipc::{persist_and_notify, SharedState};
use crate::service::DaemonEvent;

pub fn set_lcd_media(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    device_id: String,
    config: LcdConfig,
) -> IpcResponse {
    let mut state = state.lock();
    let app_config = state.config.get_or_insert_with(AppConfig::default);
    if let Some(lcd) = app_config
        .lcds
        .iter_mut()
        .find(|l| l.device_id() == device_id)
    {
        *lcd = config;
    } else {
        app_config.lcds.push(config);
    }
    persist_and_notify(&mut state, &tx, "SetLcdMedia")
}

pub fn set_fan_config(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    config: FanConfig,
) -> IpcResponse {
    let mut state = state.lock();
    state.config.get_or_insert_with(AppConfig::default).fans = Some(config);
    persist_and_notify(&mut state, &tx, "SetFanConfig")
}

pub fn set_fan_speed(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    device_index: u8,
    fan_pwm: [u8; 4],
) -> IpcResponse {
    debug!("SetFanSpeed for device {device_index}: {fan_pwm:?}");
    let mut state = state.lock();

    // Resolve device_index to a device_id from the live device list so the
    // fan controller can route speeds to the correct wired or wireless device.
    let resolved_id = state
        .devices
        .get(device_index as usize)
        .map(|d| d.device_id.clone());

    let fans = state
        .config
        .get_or_insert_with(AppConfig::default)
        .fans
        .get_or_insert_with(Default::default);
    let idx = device_index as usize;
    while fans.speeds.len() <= idx {
        fans.speeds.push(FanGroup {
            device_id: None,
            speeds: [
                FanSpeed::Constant(128),
                FanSpeed::Constant(128),
                FanSpeed::Constant(128),
                FanSpeed::Constant(128),
            ],
        });
    }
    // Populate device_id if missing so the fan controller can route to the correct device.
    if fans.speeds[idx].device_id.is_none() {
        fans.speeds[idx].device_id = resolved_id.clone();
    }
    fans.speeds[idx].speeds = [
        FanSpeed::Constant(fan_pwm[0]),
        FanSpeed::Constant(fan_pwm[1]),
        FanSpeed::Constant(fan_pwm[2]),
        FanSpeed::Constant(fan_pwm[3]),
    ];
    persist_and_notify(&mut state, &tx, "SetFanSpeed")
}

pub fn set_rgb_config(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    config: RgbAppConfig,
) -> IpcResponse {
    let mut state = state.lock();
    state.config.get_or_insert_with(AppConfig::default).rgb = Some(config);
    persist_and_notify(&mut state, &tx, "SetRgbConfig")
}
