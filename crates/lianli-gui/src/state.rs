//! Shared application state, updated by the backend thread and read by UI callbacks.

use lianli_shared::config::AppConfig;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::rgb::RgbDeviceCapabilities;
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::LcdTemplate;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Actions that are in-flight on a device card. Cleared when the daemon
/// reports the expected state change, or after a safety timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    Bind,
    Unbind,
    SwitchDisplay,
}

impl PendingAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::Unbind => "unbind",
            Self::SwitchDisplay => "switch",
        }
    }
}

pub const PENDING_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct SharedState {
    pub config: Option<AppConfig>,
    pub rgb_caps: Vec<RgbDeviceCapabilities>,
    pub devices: Vec<DeviceInfo>,
    pub available_sensors: Vec<SensorInfo>,
    /// Combined built-in + user LCD templates, fetched via `GetLcdTemplates`.
    pub lcd_templates: Vec<LcdTemplate>,
    /// device_id → (action, started_at). Populated on button click, expired
    /// after PENDING_TIMEOUT.
    pub pending_actions: HashMap<String, (PendingAction, Instant)>,
}

impl SharedState {
    pub fn set_pending(&mut self, device_id: impl Into<String>, action: PendingAction) {
        self.pending_actions
            .insert(device_id.into(), (action, Instant::now()));
    }

    pub fn expire_pending(&mut self, devices: &[DeviceInfo]) {
        let now = Instant::now();
        self.pending_actions.retain(|key, (action, started)| {
            if now.duration_since(*started) >= PENDING_TIMEOUT {
                return false;
            }
            match action {
                PendingAction::Bind => {
                    let Some(mac) = key.strip_prefix("wireless-unbound:") else {
                        return true;
                    };
                    let bound_id = format!("wireless:{mac}");
                    !devices.iter().any(|d| d.device_id == bound_id)
                }
                PendingAction::Unbind | PendingAction::SwitchDisplay => {
                    devices.iter().any(|d| &d.device_id == key)
                }
            }
        });
    }
}
