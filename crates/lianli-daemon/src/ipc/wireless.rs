//! Wireless IPC handlers: `BindWirelessDevice`, `UnbindWirelessDevice`.

use std::sync::mpsc::Sender;

use super::SharedState;
use lianli_shared::ipc::{IpcResponse, WirelessOperationStatus};
use std::collections::BTreeMap;

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

pub struct WirelessOperations {
    session: String,
    next_id: u64,
    entries: BTreeMap<String, WirelessOperationStatus>,
}

impl Default for WirelessOperations {
    fn default() -> Self {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            session: format!("{:x}-{:x}", std::process::id(), started.as_nanos()),
            next_id: 0,
            entries: BTreeMap::new(),
        }
    }
}

impl WirelessOperations {
    fn reserve(&mut self) -> anyhow::Result<String> {
        if self.entries.len() >= 64 {
            let completed = self.entries.iter().find_map(|(id, status)| {
                (!matches!(status, WirelessOperationStatus::Pending)).then(|| id.clone())
            });
            let id =
                completed.ok_or_else(|| anyhow::anyhow!("wireless operation queue is full"))?;
            self.entries.remove(&id);
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("wireless operation IDs exhausted"))?;
        let id = format!("{}-{}", self.session, self.next_id);
        self.entries
            .insert(id.clone(), WirelessOperationStatus::Pending);
        Ok(id)
    }

    pub(crate) fn complete(&mut self, id: &str, result: anyhow::Result<()>) {
        if let Some(status) = self.entries.get_mut(id) {
            *status = match result {
                Ok(()) => WirelessOperationStatus::Succeeded,
                Err(error) => WirelessOperationStatus::Failed {
                    message: format!("{error:#}"),
                },
            };
        }
    }
}

pub fn operation(state: &SharedState, id: String) -> IpcResponse {
    match state.lock().wireless_operations.entries.get(&id) {
        Some(status) => IpcResponse::ok(status),
        None => IpcResponse::error("wireless operation not found or expired"),
    }
}

pub fn bind(state: &SharedState, tx: Sender<DaemonEvent>, mac: String) -> IpcResponse {
    queue_binding(state, tx, mac, true)
}

pub fn unbind(state: &SharedState, tx: Sender<DaemonEvent>, mac: String) -> IpcResponse {
    queue_binding(state, tx, mac, false)
}

fn queue_binding(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    mac: String,
    bind: bool,
) -> IpcResponse {
    if parse_mac(&format!("wireless:{mac}")).is_none() {
        return IpcResponse::error("invalid wireless MAC address");
    }
    let mut state = state.lock();
    let operation_id = match state.wireless_operations.reserve() {
        Ok(id) => id,
        Err(error) => return IpcResponse::error(error.to_string()),
    };
    let event = if bind {
        DaemonEvent::Bind {
            mac_address: mac,
            operation_id: operation_id.clone(),
        }
    } else {
        DaemonEvent::Unbind {
            mac_address: mac,
            operation_id: operation_id.clone(),
        }
    };
    if tx.send(event).is_err() {
        state.wireless_operations.entries.remove(&operation_id);
        return IpcResponse::error("wireless command queue is disconnected");
    }
    IpcResponse::ok(
        serde_json::json!({ "operation_id": operation_id, "message": "Wireless command queued." }),
    )
}

pub fn reboot_lcd(tx: Sender<DaemonEvent>, device_id: String) -> IpcResponse {
    let Some(mac) = parse_mac(&device_id) else {
        return IpcResponse::error("invalid device_id format");
    };
    if tx.send(DaemonEvent::RebootWirelessLcd { mac }).is_err() {
        return IpcResponse::error("wireless command queue is disconnected");
    }
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
    if tx
        .send(DaemonEvent::DisableLc217Wifi { mac, disable })
        .is_err()
    {
        return IpcResponse::error("wireless command queue is disconnected");
    }
    IpcResponse::ok(serde_json::json!({"message": "LC217 wifi toggle queued."}))
}

pub fn bind_all(tx: Sender<DaemonEvent>) -> IpcResponse {
    if tx.send(DaemonEvent::BindAll).is_err() {
        return IpcResponse::error("wireless command queue is disconnected");
    }
    IpcResponse::ok(serde_json::json!({"message": "Bind all queued."}))
}

pub fn unbind_all(tx: Sender<DaemonEvent>) -> IpcResponse {
    if tx.send(DaemonEvent::UnbindAll).is_err() {
        return IpcResponse::error("wireless command queue is disconnected");
    }
    IpcResponse::ok(serde_json::json!({"message": "Unbind all queued."}))
}

pub fn get_channel(state: &std::sync::Arc<parking_lot::Mutex<super::DaemonState>>) -> IpcResponse {
    let state = state.lock();
    if let Some(dev) = state
        .devices
        .iter()
        .find(|d| d.device_id.starts_with("wireless:"))
    {
        IpcResponse::ok(serde_json::json!({"channel": dev.serial}))
    } else {
        IpcResponse::error("no wireless device found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_do_not_alias_across_daemon_sessions() {
        let mut previous = WirelessOperations {
            session: "previous".into(),
            ..Default::default()
        };
        let mut current = WirelessOperations {
            session: "current".into(),
            ..Default::default()
        };
        let previous_id = previous.reserve().unwrap();
        let current_id = current.reserve().unwrap();
        assert_ne!(previous_id, current_id);
        assert!(!current.entries.contains_key(&previous_id));
    }

    #[test]
    fn operation_results_are_bounded_and_pending_work_is_never_evicted() {
        let mut operations = WirelessOperations::default();
        let first = operations.reserve().unwrap();
        for _ in 1..64 {
            operations.reserve().unwrap();
        }
        assert!(operations.reserve().is_err());
        assert!(matches!(
            operations.entries[&first],
            WirelessOperationStatus::Pending
        ));
        operations.complete(&first, Err(anyhow::anyhow!("receiver timed out")));
        let result = serde_json::to_value(&operations.entries[&first]).unwrap();
        assert_eq!(
            result,
            serde_json::json!({"status": "failed", "message": "receiver timed out"})
        );
        let next = operations.reserve().unwrap();
        assert!(!operations.entries.contains_key(&first));
        assert_eq!(operations.entries.len(), 64);
        operations.complete(&next, Ok(()));
        assert!(matches!(
            operations.entries[&next],
            WirelessOperationStatus::Succeeded
        ));
    }

    #[test]
    fn disconnected_queue_does_not_report_acceptance_or_retain_pending_operation() {
        let state = std::sync::Arc::new(parking_lot::Mutex::new(super::super::DaemonState::new(
            std::path::PathBuf::from("/tmp/lianli-wireless-ipc-test/config.json"),
        )));
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let result = bind(&state, tx, "01:02:03:04:05:06".into());
        assert!(matches!(result, IpcResponse::Error { .. }));
        assert!(state.lock().wireless_operations.entries.is_empty());
    }
}
