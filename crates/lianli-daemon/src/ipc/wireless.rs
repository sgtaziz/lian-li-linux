//! Wireless IPC handlers: `BindWirelessDevice`, `UnbindWirelessDevice`.

use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;

use crate::service::DaemonEvent;

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
