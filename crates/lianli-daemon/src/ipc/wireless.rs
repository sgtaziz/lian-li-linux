//! Wireless IPC handlers: `BindWirelessDevice`, `UnbindWirelessDevice`.

use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;

use crate::service::DaemonEvent;

fn parse_mac(device_id: &str) -> Option<[u8; 6]> {
    let hex: Vec<&str> = device_id.strip_prefix("wireless:")?.split(':').collect();
    if hex.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, h) in hex.iter().enumerate() {
        mac[i] = u8::from_str_radix(h, 16).ok()?;
    }
    Some(mac)
}

pub fn bind(tx: Sender<DaemonEvent>, mac: String) -> IpcResponse {
    let _ = tx.send(DaemonEvent::Bind { mac_address: mac });
    IpcResponse::ok(serde_json::json!({
        "message": "Bind command queued. Device should appear shortly."
    }))
}

pub fn unbind(tx: Sender<DaemonEvent>, mac: String) -> IpcResponse {
    let _ = tx.send(DaemonEvent::Unbind { mac_address: mac });
    IpcResponse::ok(serde_json::json!({
        "message": "Unbind command queued."
    }))
}

pub fn reboot_lcd(tx: Sender<DaemonEvent>, device_id: String) -> IpcResponse {
    let Some(mac) = parse_mac(&device_id) else {
        return IpcResponse::error("invalid device_id format");
    };
    let _ = tx.send(DaemonEvent::RebootWirelessLcd { mac });
    IpcResponse::ok(serde_json::json!({"message": "LCD reboot queued."}))
}

pub fn disable_lc217_wifi(
    tx: Sender<DaemonEvent>,
    device_id: String,
    disable: bool,
) -> IpcResponse {
    let Some(mac) = parse_mac(&device_id) else {
        return IpcResponse::error("invalid device_id format");
    };
    let _ = tx.send(DaemonEvent::DisableLc217Wifi { mac, disable });
    IpcResponse::ok(serde_json::json!({"message": "LC217 wifi toggle queued."}))
}

pub fn bind_all(tx: Sender<DaemonEvent>) -> IpcResponse {
    let _ = tx.send(DaemonEvent::BindAll);
    IpcResponse::ok(serde_json::json!({"message": "Bind all queued."}))
}

pub fn unbind_all(tx: Sender<DaemonEvent>) -> IpcResponse {
    let _ = tx.send(DaemonEvent::UnbindAll);
    IpcResponse::ok(serde_json::json!({"message": "Unbind all queued."}))
}

pub fn get_channel(state: &std::sync::Arc<parking_lot::Mutex<super::DaemonState>>) -> IpcResponse {
    let state = state.lock();
    if let Some(ref dev) = state
        .devices
        .iter()
        .find(|d| d.device_id.starts_with("wireless:"))
    {
        IpcResponse::ok(serde_json::json!({"channel": dev.serial}))
    } else {
        IpcResponse::error("no wireless device found")
    }
}
