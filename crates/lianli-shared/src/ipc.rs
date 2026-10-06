use crate::config::{AppConfig, LcdConfig};
use crate::device_id::DeviceFamily;
use crate::fan::FanConfig;
use crate::rgb::{RgbAppConfig, RgbEffect};
use crate::template::catalog::CatalogTemplate;
use crate::template::LcdTemplate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Requests from GUI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum IpcRequest {
    UploadStartupImage {
        device_id: String,
        jpeg_base64: String,
    },
    GetStartupImageStatus,
    CancelStartupImage {
        id: u64,
    },
    Guarded {
        guard: crate::daemon::WriteGuard,
        request: Box<IpcRequest>,
    },
    StopService {
        invocation_id: String,
    },
    ListDevices,
    GetConfig,
    CheckMediaAccess {
        #[serde(deserialize_with = "crate::serde_limits::lcds")]
        lcds: Vec<LcdConfig>,
        #[serde(deserialize_with = "crate::serde_limits::templates")]
        templates: Vec<LcdTemplate>,
    },
    /// Replace the entire config (daemon writes to disk + reloads).
    SetConfig {
        config: Box<AppConfig>,
    },
    SetLcdMedia {
        device_id: String,
        config: Box<LcdConfig>,
    },
    SetFanConfig {
        config: FanConfig,
    },
    GetTelemetry,
    /// Get RGB capabilities for all devices.
    GetRgbCapabilities,
    /// Set RGB effect for a specific device zone. Software effects acknowledge
    /// acceptance by the bounded renderer; device delivery runs asynchronously.
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
    /// Upload a multi-frame animation to a wireless device. The firmware stores
    /// the frames and loops them onboard at `interval_ms`, so no further host
    /// packets are needed (unlike streamed Direct mode). Each frame is a
    /// full-device LED buffer (one `[r, g, b]` per LED).
    SetRgbFrames {
        device_id: String,
        frames: Vec<Vec<[u8; 3]>>,
        interval_ms: u16,
    },
    /// Set a single LED's color by index within a zone.
    /// Convenience wrapper around SetRgbDirect that modifies one LED
    /// without requiring the caller to track the full zone state.
    SetLedColor {
        device_id: String,
        zone: u8,
        led_index: u16,
        color: [u8; 3],
    },
    /// Save current direct-mode colors as a named preset.
    SaveRgbPreset {
        name: String,
        device_id: String,
    },
    /// Delete a named RGB preset. Scoped by (name, device_id) to match save.
    DeleteRgbPreset {
        name: String,
        device_id: String,
    },
    /// Get current per-LED colors for a wireless device zone.
    GetZoneColors {
        device_id: String,
        zone: u8,
    },
    /// List all saved RGB presets.
    ListRgbPresets,
    /// Apply a named preset (sends stored colors to device).
    /// Scoped by (name, device_id) to avoid ambiguity when multiple devices
    /// share the same preset name.
    ApplyRgbPreset {
        name: String,
        device_id: String,
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
    RetryDesktopDisplay {
        bus: u8,
        address: u8,
        product_id: u16,
    },
    RetryOpenRgb,
    RetryMedia,
    ClearStartupImageRecovery {
        device_id: String,
    },
    ListStateBackups,
    PreviewStateBackup {
        target: crate::backups::BackupTarget,
        #[serde(default)]
        preserved: bool,
    },
    DeleteStateBackup {
        target: crate::backups::BackupTarget,
        #[serde(default)]
        preserved: bool,
        sha256: String,
    },
    RestoreStateBackup {
        target: crate::backups::BackupTarget,
        sha256: String,
    },
    GetWirelessOperation {
        operation_id: String,
    },
    BindWirelessDevice {
        mac: String,
    },
    /// Unbind a wireless device from this dongle.
    UnbindWirelessDevice {
        mac: String,
    },
    /// Override the daisy-chain fan quantity for an ENE 6K77 port.
    SetEne6k77FanQuantity {
        device_id: String,
        quantity: u8,
    },
    ListSensors,
    ListPwmHeaders,
    GetLcdTemplates,
    SetLcdTemplates {
        #[serde(deserialize_with = "crate::serde_limits::templates")]
        templates: Vec<LcdTemplate>,
    },
    MergeLcdTemplates {
        #[serde(deserialize_with = "crate::serde_limits::templates")]
        originals: Vec<LcdTemplate>,
        #[serde(deserialize_with = "crate::serde_limits::templates")]
        copies: Vec<LcdTemplate>,
    },
    InstallTemplate {
        template: CatalogTemplate,
    },
    StartCatalogInstall {
        template: CatalogTemplate,
    },
    GetCatalogInstallStatus,
    GetCatalogStorage,
    GetManagedMediaStorage,
    StartManagedMediaReview {
        directory: String,
    },
    GetManagedMediaReview {
        operation_id: String,
    },
    StartManagedMediaRemoval {
        operation_id: String,
    },
    StartCatalogReview {
        directory: String,
    },
    GetCatalogReview {
        operation_id: String,
    },
    StartCatalogRemoval {
        operation_id: String,
    },
    /// Returns `{ "jpeg_base64": "..." }` for use in the editor preview.
    RenderTemplatePreview {
        template: LcdTemplate,
        width: u32,
        height: u32,
    },
    /// Returns `{ "jpeg_base64": "..." }` for an unsaved sensor gauge entry.
    RenderSensorPreview {
        lcd: Box<LcdConfig>,
        width: u32,
        height: u32,
    },
    Ping,
    GetDaemonInfo,
    GetInstallationHealth,
    SetLcdBrightness {
        device_id: String,
        brightness: u8,
        #[serde(default)]
        request_id: Option<String>,
    },
    PingDevice {
        device_id: String,
        zone: u8,
    },
    RebootWirelessLcd {
        device_id: String,
    },
    DisableLc217Wifi {
        device_id: String,
        disable: bool,
    },
    BindAllWireless,
    UnbindAllWireless,
    GetChannel,
    SetMergeLightingConfig {
        config: crate::rgb::MergeLightingConfig,
    },
    GetMergeLightingConfig,
    SaveDeviceProfile {
        name: String,
        device_id: String,
    },
    DeleteDeviceProfile {
        name: String,
    },
    ListDeviceProfiles,
    ApplyDeviceProfile {
        name: String,
        device_id: String,
    },
    /// Run pixel conditioning / exercise loop to clear image retention on LCD(s).
    StartPixelClean {
        #[serde(default)]
        device_id: Option<String>,
        #[serde(default = "default_pixel_clean_minutes")]
        duration_minutes: u16,
        #[serde(default)]
        preparation_id: Option<u64>,
    },
    /// Stop pixel conditioning loop and restore previous LCD configuration.
    StopPixelClean {
        #[serde(default)]
        device_id: Option<String>,
        #[serde(default)]
        session_id: Option<u64>,
    },
    /// Query current pixel cleaner status.
    GetPixelCleanStatus,
    GetPixelCleanPreparation {
        session_id: u64,
    },
}

impl IpcRequest {
    pub fn is_read_only(&self) -> bool {
        match self {
            Self::ListDevices
            | Self::GetConfig
            | Self::CheckMediaAccess { .. }
            | Self::GetTelemetry
            | Self::GetRgbCapabilities
            | Self::GetZoneColors { .. }
            | Self::ListRgbPresets
            | Self::GetWirelessOperation { .. }
            | Self::ListSensors
            | Self::ListPwmHeaders
            | Self::GetLcdTemplates
            | Self::RenderTemplatePreview { .. }
            | Self::RenderSensorPreview { .. }
            | Self::Ping
            | Self::GetDaemonInfo
            | Self::GetInstallationHealth
            | Self::GetChannel
            | Self::GetMergeLightingConfig
            | Self::ListDeviceProfiles
            | Self::GetPixelCleanStatus
            | Self::GetStartupImageStatus
            | Self::GetPixelCleanPreparation { .. }
            | Self::ListStateBackups
            | Self::GetCatalogInstallStatus
            | Self::GetCatalogStorage
            | Self::GetManagedMediaStorage
            | Self::StartManagedMediaReview { .. }
            | Self::GetManagedMediaReview { .. }
            | Self::StartCatalogReview { .. }
            | Self::GetCatalogReview { .. }
            | Self::PreviewStateBackup { .. } => true,
            Self::Guarded { .. }
            | Self::StartCatalogRemoval { .. }
            | Self::StartManagedMediaRemoval { .. }
            | Self::MergeLcdTemplates { .. }
            | Self::RestoreStateBackup { .. }
            | Self::DeleteStateBackup { .. }
            | Self::StopService { .. }
            | Self::SetConfig { .. }
            | Self::SetLcdMedia { .. }
            | Self::SetFanConfig { .. }
            | Self::SetRgbEffect { .. }
            | Self::SetRgbDirect { .. }
            | Self::SetRgbFrames { .. }
            | Self::SetLedColor { .. }
            | Self::SaveRgbPreset { .. }
            | Self::DeleteRgbPreset { .. }
            | Self::ApplyRgbPreset { .. }
            | Self::SetMbRgbSync { .. }
            | Self::SetFanDirection { .. }
            | Self::SetRgbConfig { .. }
            | Self::SwitchDisplayMode { .. }
            | Self::RetryDesktopDisplay { .. }
            | Self::RetryOpenRgb
            | Self::RetryMedia
            | Self::ClearStartupImageRecovery { .. }
            | Self::BindWirelessDevice { .. }
            | Self::UnbindWirelessDevice { .. }
            | Self::SetEne6k77FanQuantity { .. }
            | Self::SetLcdTemplates { .. }
            | Self::InstallTemplate { .. }
            | Self::StartCatalogInstall { .. }
            | Self::SetLcdBrightness { .. }
            | Self::PingDevice { .. }
            | Self::RebootWirelessLcd { .. }
            | Self::DisableLc217Wifi { .. }
            | Self::BindAllWireless
            | Self::UnbindAllWireless
            | Self::SetMergeLightingConfig { .. }
            | Self::SaveDeviceProfile { .. }
            | Self::DeleteDeviceProfile { .. }
            | Self::ApplyDeviceProfile { .. }
            | Self::StartPixelClean { .. }
            | Self::StopPixelClean { .. }
            | Self::UploadStartupImage { .. }
            | Self::CancelStartupImage { .. } => false,
        }
    }

    pub fn authorize(self, daemon: &crate::daemon::DaemonInfo) -> Result<Self, String> {
        match self {
            Self::Guarded { guard, request } => {
                guard.validate(daemon)?;
                if matches!(*request, Self::Guarded { .. }) {
                    return Err("Nested guarded requests are not supported".into());
                }
                Ok(*request)
            }
            request if request.is_read_only() => Ok(request),
            _ => Err("Changes require a compatible client. Update the GUI/client and daemon together, then restart the selected daemon cleanly.".into()),
        }
    }
}

fn default_pixel_clean_minutes() -> u16 {
    30
}

/// Maximum pixel cleaner duration in minutes (255m / ~4.25h).
pub const MAX_CLEAN_MINUTES: u16 = 255;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WirelessOperationStatus {
    Pending,
    Succeeded,
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceTemperature {
    pub name: String,
    pub celsius: Option<f32>,
    pub abnormal: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceTelemetry {
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub product_type: Option<u8>,
    pub product_subtype: Option<u8>,
    pub age_ms: Option<u64>,
    pub temperatures: Vec<DeviceTemperature>,
    pub error: Option<String>,
}

/// Info about a connected device, returned by ListDevices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_image: Option<crate::startup_image::StartupImageCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<DeviceTelemetry>,
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
    #[serde(default)]
    pub pump_mb_sync_support: bool,
    pub rgb_zone_count: Option<u8>,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
    #[serde(default)]
    pub is_unbound_wireless: bool,
    /// Bind state for unbound wireless devices: "ready_to_bind" when the
    /// device reports no master, "bind_other" when it belongs to another
    /// controller. None for bound or wired devices.
    #[serde(default)]
    pub wireless_bind_status: Option<String>,
    /// True when the master owning a "bind_other" device is currently on
    /// air, meaning the daemon refuses to bind or unbind it.
    #[serde(default)]
    pub foreign_master_online: bool,
    /// Target pump RPM range (min, max) for wireless AIOs. None for non-AIO devices.
    #[serde(default)]
    pub pump_rpm_range: Option<(u32, u32)>,
    #[serde(default)]
    pub fan_quantity: Option<u8>,
    #[serde(default)]
    pub max_fan_quantity: Option<u8>,
    #[serde(default)]
    pub firmware_version: Option<String>,
    #[serde(default)]
    pub supports_c_command: bool,
    /// (port, fan_index) for daisy-chained TL LCD fans. None for other devices.
    #[serde(default)]
    pub port_index: Option<(u8, u8)>,
    /// MAC of the bound wireless group this wired device duplicates.
    /// Set via V2 dongle HID topology matching for LCD fans or via the
    /// device link MAC for receivers and AIOs. The GUI hides tagged entries.
    #[serde(default)]
    pub wireless_group_mac: Option<String>,
    #[serde(default)]
    pub topology_key: Option<String>,
}

/// Status of the OpenRGB SDK server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenRgbServerStatus {
    /// Whether the server is enabled in config.
    pub enabled: bool,
    /// Whether the server is currently listening for connections.
    pub running: bool,
    /// Configured port of the last startup attempt.
    pub port: Option<u16>,
    /// Error message if the server failed to start.
    pub error: Option<String>,
}

/// Current state of the LCD pixel conditioning cleaner.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PixelCleanStatus {
    pub active: bool,
    #[serde(default)]
    pub session_id: Option<u64>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub duration_minutes: u16,
    #[serde(default)]
    pub remaining_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPreparationState {
    WaitingForDevice,
    Preparing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPreparationStatus {
    #[serde(default)]
    pub startup_recovery_required: bool,
    pub generation: u64,
    pub device_id: String,
    pub state: MediaPreparationState,
    pub error: Option<String>,
    #[serde(default)]
    pub runtime: Option<MediaRuntimeStatus>,
    #[serde(default)]
    pub last_playback_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaRuntimeStage {
    Failed,
    StartingSource,
    AutonomousSourceConfigured,
    FrameSubmitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRuntimeStatus {
    pub stage: MediaRuntimeStage,
    pub fps_limit: f32,
    pub hardware_video_allowed: bool,
    pub fallback_reason: Option<String>,
    #[serde(default)]
    pub encoder: Option<MediaEncoderStatus>,
    #[serde(default)]
    pub h264_transfer_started: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaEncoderStatus {
    pub name: String,
    pub software_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStreamState {
    WaitingForSession,
    Starting,
    Streaming,
    Paused,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopStreamStatus {
    pub bus: u8,
    pub address: u8,
    pub product_id: u16,
    pub state: DesktopStreamState,
    pub backend: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
    pub applied_generation: Option<u64>,
    pub applied_policy: Option<crate::display::DisplayVideoPolicy>,
    #[serde(default)]
    pub encoding: Option<crate::display::DesktopEncodingStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    #[serde(default)]
    pub lcd_brightness: HashMap<String, LcdBrightnessStatus>,
    #[serde(default)]
    pub desktop_streams: Vec<DesktopStreamStatus>,
    #[serde(default)]
    pub media_preparation: HashMap<usize, MediaPreparationStatus>,
    /// Fan RPMs keyed by device_id.
    pub fan_rpms: HashMap<String, Vec<u16>>,
    /// Coolant temperatures keyed by device_id.
    pub coolant_temps: HashMap<String, f32>,
    /// Whether the daemon is actively streaming frames to LCD devices.
    pub streaming_active: bool,
    /// OpenRGB SDK server status.
    #[serde(default)]
    pub openrgb_status: OpenRgbServerStatus,
    /// Map of active pixel cleaner sessions per target (keyed by target ID, card index, or "all") to support independent concurrent sessions.
    #[serde(default)]
    pub pixel_clean_statuses: HashMap<String, PixelCleanStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LcdBrightnessStatus {
    #[serde(default)]
    pub request_id: Option<String>,
    pub brightness: u8,
    /// Waiting for driver submission, not confirmation from the panel.
    pub pending: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod quantity_tests {
    use super::*;

    #[test]
    fn brightness_status_is_optional_for_older_daemons() {
        let mut value = serde_json::to_value(TelemetrySnapshot::default()).unwrap();
        value.as_object_mut().unwrap().remove("lcd_brightness");
        let snapshot: TelemetrySnapshot = serde_json::from_value(value).unwrap();
        assert!(snapshot.lcd_brightness.is_empty());
        let failure = LcdBrightnessStatus {
            request_id: Some("brightness-request".into()),
            brightness: 30,
            pending: false,
            error: Some("USB write failed".into()),
        };
        let encoded = serde_json::to_value(failure).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "request_id": "brightness-request",
                "brightness": 30, "pending": false, "error": "USB write failed"
            })
        );
        let request: IpcRequest = serde_json::from_value(serde_json::json!({
            "method": "SetLcdBrightness", "params": { "device_id": "lcd", "brightness": 30 }
        }))
        .unwrap();
        assert!(matches!(
            request,
            IpcRequest::SetLcdBrightness {
                request_id: None,
                ..
            }
        ));
        let status: LcdBrightnessStatus = serde_json::from_value(serde_json::json!({
            "brightness": 30, "pending": false, "error": null
        }))
        .unwrap();
        assert!(status.request_id.is_none());
    }

    #[test]
    fn set_config_accepts_numeric_ene_port_keys_from_gui_and_wire() {
        let mut config = serde_json::to_value(AppConfig::default()).unwrap();
        config["ene6k77"] =
            serde_json::json!({"6243168001": {"fan_quantities": {"0": 0, "1": 4, "2": 3, "3": 1}}});
        let request = serde_json::json!({"method": "SetConfig", "params": {"config": config}});
        for parsed in [
            serde_json::from_value::<IpcRequest>(request.clone()),
            serde_json::from_str::<IpcRequest>(&request.to_string()),
        ] {
            let IpcRequest::SetConfig { config } = parsed.unwrap() else {
                panic!("wrong request")
            };
            assert_eq!(
                config.ene6k77["6243168001"].fan_quantities,
                HashMap::from([(0, 0), (1, 4), (2, 3), (3, 1)])
            );
        }
        let mut malformed = request;
        malformed["params"]["config"]["ene6k77"]["6243168001"]["fan_quantities"] =
            serde_json::json!({"hid:6243168001:port2": 3});
        assert!(serde_json::from_value::<IpcRequest>(malformed).is_err());
    }
}
