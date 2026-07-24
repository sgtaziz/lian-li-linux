//! Config-mutating IPC handlers: `SetLcdMedia`, `SetFanConfig`, `SetRgbConfig`.
//! Each updates `state.config` in place and then calls
//! [`super::persist_and_notify`] to flush to disk + wake the daemon.

use std::sync::mpsc::Sender;

use lianli_shared::config::AppConfig;
use lianli_shared::config::LcdConfig;
use lianli_shared::fan::FanConfig;
use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::RgbAppConfig;

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

pub fn set_rgb_config(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    config: RgbAppConfig,
) -> IpcResponse {
    let mut state = state.lock();
    state.config.get_or_insert_with(AppConfig::default).rgb = Some(config);
    persist_and_notify(&mut state, &tx, "SetRgbConfig")
}
