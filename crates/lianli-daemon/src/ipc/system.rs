//! System-level queries: `Ping`, `ListSensors`, `ListDevices`, `GetConfig`,
//! `GetTelemetry`.

use std::sync::Arc;

use lianli_shared::ipc::IpcResponse;
use parking_lot::Mutex;

use crate::ipc::SharedState;

pub fn ping() -> IpcResponse {
    IpcResponse::ok(serde_json::json!("pong"))
}

pub fn list_sensors(state: &SharedState) -> IpcResponse {
    let mut sensors = lianli_shared::sensors::enumerate_sensors();
    // Add wireless coolant sensors from live telemetry
    let ipc_state = state.lock();
    for (device_id, temp) in &ipc_state.telemetry.coolant_temps {
        let display = ipc_state
            .devices
            .iter()
            .find(|d| d.device_id == *device_id)
            .map(|d| format!("{} (Coolant)", d.name))
            .unwrap_or_else(|| format!("{device_id} (Coolant)"));
        sensors.push(lianli_shared::sensors::SensorInfo {
            source: lianli_shared::sensors::SensorSource::WirelessCoolant {
                device_id: device_id.clone(),
            },
            sensor_name: None,
            display_name: Some(display),
            divider: 1,
            unit: lianli_shared::sensors::Unit::C,
            current_value: Some(*temp),
        });
    }
    IpcResponse::ok(&sensors)
}

pub fn list_devices(state: &SharedState) -> IpcResponse {
    let ipc_state = state.lock();
    IpcResponse::ok(&ipc_state.devices)
}

pub fn get_config(state: &SharedState) -> IpcResponse {
    let ipc_state = state.lock();
    IpcResponse::ok(&ipc_state.config)
}

pub fn get_telemetry(state: &SharedState) -> IpcResponse {
    let ipc_state = state.lock();
    IpcResponse::ok(&ipc_state.telemetry)
}

#[allow(dead_code)]
fn _suppress_unused(_: Arc<Mutex<()>>) {}
