use crate::config::{AppConfig, LcdConfig};
use crate::device_id::DeviceFamily;
use crate::fan::FanConfig;
use crate::rgb::{RgbAppConfig, RgbEffect};
use crate::template::LcdTemplate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Requests from GUI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum IpcRequest {
    ListDevices,
    GetConfig,
    /// Replace the entire config (daemon writes to disk + reloads).
    SetConfig {
        config: AppConfig,
    },
    SetLcdMedia {
        device_id: String,
        config: LcdConfig,
    },
    SetFanSpeed {
        device_index: u8,
        fan_pwm: [u8; 4],
    },
    SetFanConfig {
        config: FanConfig,
    },
    GetTelemetry,
    /// Get RGB capabilities for all devices.
    GetRgbCapabilities,
    /// Set RGB effect for a specific device zone.
    SetRgbEffect {
        device_id: String,
        zone: u8,
        effect: RgbEffect,
    },
    /// Set per-LED colors directly (used by OpenRGB integration).
    SetRgbDirect {
        device_id: String,
        zone: u8,
        /// RGB triplets, one per LED.
        colors: Vec<[u8; 3]>,
    },
    /// Enable/disable motherboard ARGB sync for a device.
    SetMbRgbSync {
        device_id: String,
        enabled: bool,
    },
    /// Set fan direction (swap LR/TB) for a device zone.
    SetFanDirection {
        device_id: String,
        zone: u8,
        swap_lr: bool,
        swap_tb: bool,
    },
    /// Update the RGB configuration section.
    SetRgbConfig {
        config: RgbAppConfig,
    },
    /// Switch a WinUSB LCD device between LCD mode and desktop mode.
    SwitchDisplayMode {
        device_id: String,
    },
    /// Bind an unbound wireless device to this dongle.
    BindWirelessDevice {
        mac: String,
    },
    ListSensors,
    GetLcdTemplates,
    SetLcdTemplates {
        templates: Vec<LcdTemplate>,
    },
    /// Returns `{ "jpeg_base64": "..." }` for use in the editor preview.
    RenderTemplatePreview {
        template: LcdTemplate,
        width: u32,
        height: u32,
    },
    Subscribe,
    Ping,
}

/// Responses from daemon to GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum IpcResponse {
    #[serde(rename = "ok")]
    Ok { data: serde_json::Value },
    #[serde(rename = "error")]
    Error { message: String },
}

impl IpcResponse {
    pub fn ok(data: impl Serialize) -> Self {
        Self::Ok {
            data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self::Error {
            message: msg.into(),
        }
    }
}

/// Event notifications pushed from daemon to subscribed clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum IpcEvent {
    DeviceAttached {
        device_id: String,
        family: DeviceFamily,
        name: String,
    },
    DeviceDetached {
        device_id: String,
    },
    ConfigChanged,
    FanSpeedUpdate {
        device_index: u8,
        rpms: Vec<u16>,
    },
}

/// Info about a connected device, returned by ListDevices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub family: DeviceFamily,
    pub name: String,
    pub serial: Option<String>,
    #[serde(default)]
    pub vid: u16,
    #[serde(default)]
    pub pid: u16,
    pub has_lcd: bool,
    pub has_fan: bool,
    pub has_pump: bool,
    pub has_rgb: bool,
    /// Whether this device exposes a controllable pump (speed slot 3).
    #[serde(default)]
    pub has_pump_control: bool,
    pub fan_count: Option<u8>,
    pub per_fan_control: Option<bool>,
    pub mb_sync_support: bool,
    pub rgb_zone_count: Option<u8>,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
    #[serde(default)]
    pub is_unbound_wireless: bool,
}

/// Status of the OpenRGB SDK server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenRgbServerStatus {
    /// Whether the server is enabled in config.
    pub enabled: bool,
    /// Whether the server is currently listening for connections.
    pub running: bool,
    /// The actual port the server bound to (may differ from configured if port was in use).
    pub port: Option<u16>,
    /// Error message if the server failed to start.
    pub error: Option<String>,
}

/// Snapshot of live telemetry data, returned by GetTelemetry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    /// Fan RPMs keyed by device_id.
    pub fan_rpms: HashMap<String, Vec<u16>>,
    /// Coolant temperatures keyed by device_id.
    pub coolant_temps: HashMap<String, f32>,
    /// Whether the daemon is actively streaming frames to LCD devices.
    pub streaming_active: bool,
    /// OpenRGB SDK server status.
    #[serde(default)]
    pub openrgb_status: OpenRgbServerStatus,
}
