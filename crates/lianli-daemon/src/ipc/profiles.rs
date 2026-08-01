use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;
use lianli_shared::profile::DeviceProfile;
use tracing::info;

use crate::ipc::{DaemonState, SharedState};
use crate::persistence::write_json;
use crate::service::DaemonEvent;

fn profiles_dir(state: &DaemonState) -> std::path::PathBuf {
    state
        .config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("profiles")
}

fn profile_path(state: &DaemonState, name: &str) -> std::path::PathBuf {
    profiles_dir(state).join(format!("{name}.json"))
}

fn read_all_profiles(state: &DaemonState) -> Vec<DeviceProfile> {
    let dir = profiles_dir(state);
    let mut profiles = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Ok(p) = serde_json::from_str::<DeviceProfile>(&data) {
                        profiles.push(p);
                    }
                }
            }
        }
    }
    profiles
}

pub fn save(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    name: String,
    device_id: String,
) -> IpcResponse {
    let st = state.lock();
    let device = st.devices.iter().find(|d| d.device_id == device_id);
    let family = device
        .map(|d| {
            serde_json::to_string(&d.family)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_default();
    let config = match st.config.as_ref() {
        Some(c) => c,
        None => return IpcResponse::error("no config loaded"),
    };
    let profile = DeviceProfile::capture_from_config(config, &name, &device_id, &family);
    let path = profile_path(&st, &name);
    if let Err(e) = write_json(&path, &profile) {
        return IpcResponse::error(format!("failed to write profile: {e}"));
    }
    drop(st);
    let _ = tx.send(DaemonEvent::IpcUpdate);
    info!("Device profile '{name}' saved for {device_id} ({family})");
    IpcResponse::ok(serde_json::json!(null))
}

pub fn delete(state: &SharedState, tx: Sender<DaemonEvent>, name: String) -> IpcResponse {
    let st = state.lock();
    let path = profile_path(&st, &name);
    if !path.exists() {
        return IpcResponse::error(format!("profile '{name}' not found"));
    }
    if let Err(e) = std::fs::remove_file(&path) {
        return IpcResponse::error(format!("failed to delete profile: {e}"));
    }
    drop(st);
    let _ = tx.send(DaemonEvent::IpcUpdate);
    info!("Device profile '{name}' deleted");
    IpcResponse::ok(serde_json::json!(null))
}

pub fn list(state: &SharedState) -> IpcResponse {
    let st = state.lock();
    let profiles = read_all_profiles(&st);
    let entries: Vec<serde_json::Value> = profiles
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "device_id": p.device_id,
                "device_family": p.device_family
            })
        })
        .collect();
    IpcResponse::ok(entries)
}

pub fn apply(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    name: String,
    device_id: String,
) -> IpcResponse {
    let mut st = state.lock();
    let profile = {
        let profiles = read_all_profiles(&st);
        profiles.into_iter().find(|p| p.name == name)
    };
    let Some(mut profile) = profile else {
        return IpcResponse::error(format!("profile '{name}' not found"));
    };
    profile.device_id = device_id.clone();
    let config = st.config.get_or_insert_with(Default::default);
    profile.apply_to_config(config);
    let config_snapshot = config.clone();
    let config_path = st.config_path.clone();
    drop(st);
    if let Err(e) = super::write_config(&config_path, &config_snapshot) {
        return IpcResponse::error(format!("failed to write config: {e}"));
    }
    let _ = tx.send(DaemonEvent::IpcUpdate);
    info!("Device profile '{name}' applied to {device_id}");
    IpcResponse::ok(serde_json::json!(null))
}
