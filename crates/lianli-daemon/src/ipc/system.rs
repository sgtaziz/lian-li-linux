//! System-level queries: `Ping`, `ListSensors`, `ListDevices`, `GetConfig`,
//! `GetTelemetry`.

use lianli_shared::ipc::IpcResponse;
use lianli_shared::sensors::SensorCategory;
use lianli_shared::ui_message::UiMessage;

fn refused(english: &str, key: &str) -> IpcResponse {
    IpcResponse::error_ui(english, UiMessage::new(key))
}

pub fn clear_startup_recovery(
    state: &super::SharedState,
    tx: super::EventSender,
    device_id: String,
) -> IpcResponse {
    if !state
        .lock()
        .telemetry
        .media_preparation
        .values()
        .any(|status| status.device_id == device_id && status.startup_recovery_required)
    {
        return refused(
            "This LCD has no startup recovery block. Refresh its status",
            "daemon.recovery.noStartupBlock",
        );
    }
    match tx.send(crate::service::DaemonEvent::ClearStartupImageRecovery { device_id }) {
        Ok(()) => IpcResponse::ok(serde_json::json!(null)),
        Err(_) => refused(
            "The daemon stopped before accepting recovery clearance",
            "daemon.recovery.clearNotAccepted",
        ),
    }
}

pub fn retry_media(state: &super::SharedState, tx: super::EventSender) -> IpcResponse {
    use lianli_shared::ipc::{MediaPreparationState, MediaRuntimeStage};
    let mut state = state.lock();
    if !state.telemetry.media_preparation.values().any(|status| {
        status.state == MediaPreparationState::Failed
            || status
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.stage == MediaRuntimeStage::Failed)
    }) {
        return refused(
            "No failed media is available to retry. Refresh its status",
            "daemon.recovery.noFailedMedia",
        );
    }
    if state.media_retry_pending {
        return refused(
            "A media retry is already queued",
            "daemon.recovery.mediaRetryQueued",
        );
    }
    state.media_retry_pending = true;
    if tx.send(crate::service::DaemonEvent::RetryMedia).is_err() {
        state.media_retry_pending = false;
        return refused(
            "The daemon stopped before accepting the media retry",
            "daemon.recovery.mediaRetryNotAccepted",
        );
    }
    IpcResponse::ok(serde_json::json!(null))
}

pub fn retry_openrgb(state: &super::SharedState, tx: super::EventSender) -> IpcResponse {
    let mut state = state.lock();
    let status = &state.telemetry.openrgb_status;
    if !status.enabled || status.running || status.error.is_none() {
        return refused(
            "OpenRGB is not in a failed enabled state. Refresh its status",
            "daemon.recovery.openRgbNotFailed",
        );
    }
    if state.openrgb_retry_pending {
        return refused(
            "An OpenRGB retry is already queued",
            "daemon.recovery.openRgbRetryQueued",
        );
    }
    state.openrgb_retry_pending = true;
    if tx.send(crate::service::DaemonEvent::RetryOpenRgb).is_err() {
        state.openrgb_retry_pending = false;
        return refused(
            "The daemon stopped before accepting the OpenRGB retry",
            "daemon.recovery.openRgbRetryNotAccepted",
        );
    }
    IpcResponse::ok(serde_json::json!(null))
}

use crate::ipc::SharedState;

pub fn ping() -> IpcResponse {
    IpcResponse::ok(serde_json::json!("pong"))
}

pub fn daemon_info(state: &SharedState) -> IpcResponse {
    let (mut info, gate) = {
        let state = state.lock();
        (state.info.clone(), state.write_gate.clone())
    };
    info.service_operation_lock = gate.identity().ok();
    IpcResponse::ok(info)
}

pub fn list_sensors(state: &SharedState) -> IpcResponse {
    use lianli_shared::sensors::{SensorInfo, SensorSource, Unit};
    let mut sensors = lianli_shared::sensors::enumerate_sensors();
    let ipc_state = state.lock();
    let display = |device_id: &str, kind: &str| {
        let name = ipc_state
            .devices
            .iter()
            .find(|d| d.device_id == device_id)
            .map_or(device_id, |d| d.name.as_str());
        format!("{name} ({kind})")
    };
    for (device_id, temp) in &ipc_state.telemetry.coolant_temps {
        sensors.push(SensorInfo {
            source: SensorSource::WirelessCoolant {
                device_id: device_id.clone(),
            },
            sensor_name: None,
            display_name: Some(display(device_id, "Coolant")),
            divider: 1,
            unit: Unit::C,
            current_value: Some(*temp),
        });
    }
    for (device_id, rpm) in &ipc_state.telemetry.pump_rpms {
        sensors.push(SensorInfo {
            source: SensorSource::AioPump {
                device_id: device_id.clone(),
            },
            sensor_name: None,
            display_name: Some(display(device_id, "Pump")),
            divider: 1,
            unit: Unit::RPM,
            current_value: Some(f32::from(*rpm)),
        });
    }
    IpcResponse::ok(&sensors)
}

pub fn suggest_sensor_sources(categories: &[SensorCategory]) -> IpcResponse {
    const MAX_CATEGORIES: usize = 32;
    if categories.len() > MAX_CATEGORIES {
        return IpcResponse::error_ui(
            format!("At most {MAX_CATEGORIES} sensor categories can be suggested at once"),
            UiMessage::new("daemon.sensors.tooManyCategories").param("limit", MAX_CATEGORIES),
        );
    }
    let sensors = lianli_shared::sensors::enumerate_sensors();
    let suggestions: Vec<_> = categories
        .iter()
        .map(|category| lianli_shared::sensors::pick_source_for_category(*category, &sensors))
        .collect();
    IpcResponse::ok(&suggestions)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::DaemonEvent;
    use parking_lot::Mutex;
    use std::sync::{mpsc, Arc};

    #[test]
    fn startup_recovery_clear_requires_a_block_and_queues_the_selected_device() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(crate::ipc::DaemonState::new(
            root.path().join("config.json"),
        )));
        let (tx, rx) = mpsc::channel();
        let id = "serial:hid:lcd".to_string();
        assert!(matches!(
            clear_startup_recovery(&state, tx.clone().into(), id.clone()),
            IpcResponse::Error { .. }
        ));
        let mut status: lianli_shared::ipc::MediaPreparationStatus =
            serde_json::from_value(serde_json::json!({
                "generation": 1, "device_id": id, "state": "failed", "error": "Recovery required"
            }))
            .unwrap();
        assert!(!status.startup_recovery_required);
        status.startup_recovery_required = true;
        state.lock().telemetry.media_preparation.insert(0, status);
        assert!(matches!(
            clear_startup_recovery(&state, tx.clone().into(), id.clone()),
            IpcResponse::Ok { .. }
        ));
        assert!(
            matches!(rx.try_recv().unwrap(), DaemonEvent::ClearStartupImageRecovery { device_id } if device_id == id)
        );
        drop(rx);
        assert!(matches!(
            clear_startup_recovery(&state, tx.into(), id),
            IpcResponse::Error { .. }
        ));
    }

    #[test]
    fn media_retry_requires_failure_coalesces_and_does_not_save_configuration() {
        use lianli_shared::ipc::{
            MediaPreparationState, MediaPreparationStatus, MediaRuntimeStage, MediaRuntimeStatus,
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let state = Arc::new(Mutex::new(crate::ipc::DaemonState::new(path.clone())));
        let (tx, rx) = mpsc::channel();
        assert!(matches!(
            retry_media(&state, tx.clone().into()),
            IpcResponse::Error { .. }
        ));
        state.lock().telemetry.media_preparation.insert(
            0,
            MediaPreparationStatus {
                startup_recovery_required: false,
                generation: 1,
                device_id: "fixture".into(),
                state: MediaPreparationState::Failed,
                error: Some("Missing media".into()),
                error_ui: None,
                runtime: None,
                last_playback_error: None,
                last_playback_error_ui: None,
            },
        );
        assert!(matches!(
            retry_media(&state, tx.clone().into()),
            IpcResponse::Ok { .. }
        ));
        assert!(matches!(
            retry_media(&state, tx.clone().into()),
            IpcResponse::Error { .. }
        ));
        assert!(matches!(rx.try_recv().unwrap(), DaemonEvent::RetryMedia));
        assert!(rx.try_recv().is_err());
        {
            let mut state = state.lock();
            state.media_retry_pending = false;
            let status = state.telemetry.media_preparation.get_mut(&0).unwrap();
            status.state = MediaPreparationState::Ready;
            status.runtime = Some(MediaRuntimeStatus {
                stage: MediaRuntimeStage::Failed,
                fps_limit: 30.0,
                hardware_video_allowed: false,
                fallback_reason: None,
                fallback_reason_ui: None,
                encoder: None,
                h264_transfer_started: None,
            });
        }
        assert!(matches!(
            retry_media(&state, tx.clone().into()),
            IpcResponse::Ok { .. }
        ));
        assert!(matches!(rx.try_recv().unwrap(), DaemonEvent::RetryMedia));
        state.lock().media_retry_pending = false;
        drop(rx);
        assert!(matches!(
            retry_media(&state, tx.into()),
            IpcResponse::Error { .. }
        ));
        assert!(!state.lock().media_retry_pending);
        assert!(state.lock().config.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn openrgb_retry_requires_failure_coalesces_requests_and_reports_queue_failure() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(crate::ipc::DaemonState::new(
            root.path().join("config.json"),
        )));
        let (tx, rx) = mpsc::channel();
        assert!(matches!(
            retry_openrgb(&state, tx.clone().into()),
            IpcResponse::Error { .. }
        ));
        {
            let mut state = state.lock();
            state.telemetry.openrgb_status.enabled = true;
            state.telemetry.openrgb_status.error = Some("Port in use".into());
        }
        assert!(matches!(
            retry_openrgb(&state, tx.clone().into()),
            IpcResponse::Ok { .. }
        ));
        assert!(state.lock().openrgb_retry_pending);
        assert!(matches!(
            retry_openrgb(&state, tx.clone().into()),
            IpcResponse::Error { .. }
        ));
        assert!(matches!(rx.try_recv().unwrap(), DaemonEvent::RetryOpenRgb));
        assert!(rx.try_recv().is_err());
        assert!(state.lock().config.is_none());
        state.lock().openrgb_retry_pending = false;
        drop(rx);
        assert!(matches!(
            retry_openrgb(&state, tx.into()),
            IpcResponse::Error { .. }
        ));
        assert!(!state.lock().openrgb_retry_pending);
    }
}
