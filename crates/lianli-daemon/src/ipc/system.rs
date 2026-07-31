//! System-level queries: `Ping`, `ListSensors`, `ListDevices`, `GetConfig`,
//! `GetTelemetry`.

use lianli_shared::ipc::IpcResponse;

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

pub fn list_pwm_headers() -> IpcResponse {
    let headers = lianli_shared::sensors::enumerate_pwm_headers();
    let result: Vec<serde_json::Value> = headers
        .iter()
        .map(|h| {
            let pct = lianli_shared::sensors::read_pwm_header(&h.id)
                .map(|v| (v as f32 / 255.0 * 100.0).round() as u8)
                .unwrap_or(0);
            serde_json::json!({
                "id": h.id,
                "label": format!("{} ({}%)", h.label, pct),
            })
        })
        .collect();
    IpcResponse::ok(&result)
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
