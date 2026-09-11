//! IPC server: Unix domain socket for daemon-GUI communication.
//!
//! Protocol: newline-delimited JSON (one request, one response per connection).
//! The GUI polls periodically for telemetry. Config writes go through IPC.

use crate::controllers::rgb::RgbController;
use crate::service::DaemonEvent;
use crate::template_store;
use lianli_shared::config::AppConfig;
use lianli_shared::ipc::{DeviceInfo, IpcRequest, IpcResponse, TelemetrySnapshot};
use lianli_shared::rgb::RgbPreset;
use lianli_shared::template::LcdTemplate;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use tracing::{debug, error, info, warn};

use lianli_shared::ipc::PixelCleanStatus;
use std::time::{Duration, Instant};

/// Active cleaner session state stored in DaemonState.
#[derive(Debug, Clone)]
pub struct PixelCleanState {
    pub session_id: u64,
    pub device_id: Option<String>,
    pub duration_minutes: u16,
    pub clean_until: Instant,
}

/// Shared state between the daemon main loop and the IPC server thread.
pub struct DaemonState {
    pub config: Option<AppConfig>,
    pub config_path: PathBuf,
    pub presets_path: PathBuf,
    pub devices: Vec<DeviceInfo>,
    pub telemetry: TelemetrySnapshot,
    /// RGB controller, set once devices are opened.
    pub rgb_controller: Option<Arc<Mutex<RgbController>>>,
    pub user_templates: Vec<LcdTemplate>,
    pub rgb_presets: Vec<RgbPreset>,
    pub pixel_clean_states: Vec<PixelCleanState>,
}

impl DaemonState {
    pub fn new(config_path: PathBuf) -> Self {
        let presets_path = config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("rgb_presets.json");
        let rgb_presets = crate::persistence::read_rgb_presets(&presets_path);
        Self {
            config: None,
            config_path,
            presets_path,
            devices: Vec::new(),
            telemetry: TelemetrySnapshot::default(),
            rgb_controller: None,
            user_templates: Vec::new(),
            rgb_presets,
            pixel_clean_states: Vec::new(),
        }
    }

    pub fn templates_path(&self) -> PathBuf {
        template_store::templates_path_for(&self.config_path)
    }



    /// Returns a map of all currently active pixel cleaner sessions keyed by target identifier.
    /// Supports multi-LCD setups by indexing each active cleaner under:
    /// 1. Its full target identifier (e.g. "hid:1-2:1.0#0")
    /// 2. "all" when the cleaner was triggered globally across all LCDs
    pub fn pixel_clean_statuses(&self) -> HashMap<String, PixelCleanStatus> {
        let now = Instant::now();
        let mut map = HashMap::new();
        for state in &self.pixel_clean_states {
            if now < state.clean_until {
                let remaining = (state.clean_until - now).as_secs();
                let status = PixelCleanStatus {
                    active: true,
                    session_id: Some(state.session_id),
                    device_id: state.device_id.clone(),
                    duration_minutes: state.duration_minutes,
                    remaining_seconds: remaining,
                };
                if let Some(ref dev_id) = state.device_id {
                    // Index by canonical target ID
                    map.insert(dev_id.clone(), status);
                } else {
                    // Global CLI invocation affecting all displays
                    map.insert("all".to_string(), status);
                }
            }
        }
        map
    }
}

/// Starts the IPC server in a background thread.
/// Returns the join handle for cleanup.
pub fn start_ipc_server(
    state: Arc<Mutex<DaemonState>>,
    stop_flag: Arc<AtomicBool>,
    tx: Sender<DaemonEvent>,
    socket_path: PathBuf,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_server(state, stop_flag, tx, socket_path) {
            error!("IPC server error: {e}");
        }
    })
}

fn run_server(
    state: Arc<Mutex<DaemonState>>,
    stop_flag: Arc<AtomicBool>,
    tx: Sender<DaemonEvent>,
    socket_path: PathBuf,
) -> anyhow::Result<()> {
    let socket_path = Path::new(&socket_path);
    if socket_path.exists() {
        fs::remove_file(socket_path)?;
    }

    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).ok();
    }

    let listener = UnixListener::bind(socket_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666))?;
    }

    listener.set_nonblocking(true)?;

    info!("IPC server listening on {}", socket_path.display());

    while !stop_flag.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _addr)) => {
                stream.set_nonblocking(false).ok();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();

                let state = Arc::clone(&state);
                let tx_for_client = tx.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, state, tx_for_client) {
                        debug!("IPC connection error: {e}");
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                warn!("IPC accept error: {e}");
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    fs::remove_file(socket_path).ok();
    info!("IPC server stopped");
    Ok(())
}

fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    state: Arc<Mutex<DaemonState>>,
    tx: Sender<DaemonEvent>,
) -> anyhow::Result<()> {
    let reader = BufReader::new(&stream);
    let mut writer = &stream;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: IpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = IpcResponse::error(format!("invalid request: {e}"));
                write_response(&mut writer, &resp)?;
                continue;
            }
        };

        debug!("IPC request: {request:?}");
        let response = handle_request(request, &state, tx.clone());
        write_response(&mut writer, &response)?;
    }

    Ok(())
}

fn handle_request(
    request: IpcRequest,
    state: &Arc<Mutex<DaemonState>>,
    tx: Sender<DaemonEvent>,
) -> IpcResponse {
    match request {
        IpcRequest::Ping => super::system::ping(),
        IpcRequest::ListSensors => super::system::list_sensors(state),
        IpcRequest::ListPwmHeaders => super::system::list_pwm_headers(),
        IpcRequest::ListDevices => super::system::list_devices(state),
        IpcRequest::GetConfig => super::system::get_config(state),
        IpcRequest::GetTelemetry => super::system::get_telemetry(state),

        IpcRequest::SetConfig { config } => {
            let mut state = state.lock();
            state.config = Some(config);
            super::persist_and_notify(&mut state, &tx, "SetConfig")
        }

        IpcRequest::SetLcdMedia { device_id, config } => {
            super::config::set_lcd_media(state, tx, device_id, config)
        }
        IpcRequest::SetFanConfig { config } => super::config::set_fan_config(state, tx, config),

        IpcRequest::GetRgbCapabilities => super::rgb::capabilities(state),
        IpcRequest::SetRgbEffect {
            device_id,
            zone,
            effect,
        } => super::rgb::set_effect(state, device_id, zone, effect),
        IpcRequest::SetRgbDirect {
            device_id,
            zone,
            colors,
        } => super::rgb::set_direct(state, device_id, zone, colors),
        IpcRequest::SetRgbFrames {
            device_id,
            frames,
            interval_ms,
        } => super::rgb::set_frames(state, device_id, frames, interval_ms),
        IpcRequest::SetMbRgbSync { device_id, enabled } => {
            super::rgb::set_mb_sync(state, device_id, enabled)
        }
        IpcRequest::SetFanDirection {
            device_id,
            zone,
            swap_lr,
            swap_tb,
        } => super::rgb::set_fan_direction(state, device_id, zone, swap_lr, swap_tb),
        IpcRequest::SetRgbConfig { config } => super::config::set_rgb_config(state, tx, config),

        IpcRequest::SwitchDisplayMode { device_id } => {
            super::lcd::switch_display_mode(state, tx, device_id)
        }

        IpcRequest::BindWirelessDevice { mac } => super::wireless::bind(tx, mac),
        IpcRequest::UnbindWirelessDevice { mac } => super::wireless::unbind(tx, mac),
        IpcRequest::RebootWirelessLcd { device_id } => super::wireless::reboot_lcd(tx, device_id),
        IpcRequest::DisableLc217Wifi { device_id, disable } => {
            super::wireless::disable_lc217_wifi(tx, device_id, disable)
        }
        IpcRequest::BindAllWireless => super::wireless::bind_all(tx),
        IpcRequest::UnbindAllWireless => super::wireless::unbind_all(tx),
        IpcRequest::GetChannel => super::wireless::get_channel(state),
        IpcRequest::SetMergeLightingConfig { config } => {
            let mut state = state.lock();
            if let Some(ref mut app_config) = state.config {
                app_config
                    .rgb
                    .get_or_insert_with(Default::default)
                    .merge_lighting = Some(config);
                super::persist_and_notify(&mut state, &tx, "SetMergeLightingConfig")
            } else {
                IpcResponse::error("no config loaded")
            }
        }
        IpcRequest::GetMergeLightingConfig => {
            let state = state.lock();
            let merge = state
                .config
                .as_ref()
                .and_then(|c| c.rgb.as_ref())
                .and_then(|r| r.merge_lighting.as_ref());
            IpcResponse::ok(serde_json::json!({ "config": merge }))
        }

        IpcRequest::SetEne6k77FanQuantity {
            device_id,
            quantity,
        } => super::fan::set_ene6k77_fan_quantity(tx, device_id, quantity),
        IpcRequest::SetLcdBrightness {
            device_id,
            brightness,
        } => {
            let _ = tx.send(DaemonEvent::SetLcdBrightness {
                device_id,
                brightness,
            });
            IpcResponse::ok(serde_json::json!({ "applied": true }))
        }
        IpcRequest::StartPixelClean {
            device_id,
            duration_minutes,
        } => {
            let duration_minutes =
                duration_minutes.clamp(1, lianli_shared::ipc::MAX_CLEAN_MINUTES);
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            if tx
                .send(DaemonEvent::StartPixelClean {
                    device_id,
                    duration_minutes,
                    reply: reply_tx,
                })
                .is_err()
            {
                return IpcResponse::error("daemon service not running");
            }
            match reply_rx.recv_timeout(Duration::from_secs(15)) {
                Ok(Ok(session_id)) => {
                    IpcResponse::ok(serde_json::json!({ "started": true, "session_id": session_id }))
                }
                Ok(Err(err)) => IpcResponse::error(err),
                Err(e) => IpcResponse::error(format!("timeout starting pixel cleaner: {e}")),
            }
        }
        IpcRequest::StopPixelClean {
            device_id,
            session_id,
        } => {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            if tx
                .send(DaemonEvent::StopPixelClean {
                    device_id,
                    session_id,
                    reply: Some(reply_tx),
                })
                .is_err()
            {
                return IpcResponse::error("daemon service not running");
            }
            match reply_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(stopped) => IpcResponse::ok(serde_json::json!({ "stopped": stopped })),
                Err(e) => IpcResponse::error(format!("timeout stopping pixel cleaner: {e}")),
            }
        }
        IpcRequest::GetPixelCleanStatus => {
            let state = state.lock();
            let statuses = state.pixel_clean_statuses();
            IpcResponse::ok(statuses)
        }
        IpcRequest::PingDevice { device_id, zone } => {
            let rgb = state.lock();
            if let Some(ref controller) = rgb.rgb_controller {
                match controller.lock().ping(&device_id, zone) {
                    Ok(()) => IpcResponse::ok(serde_json::json!({ "pinged": true })),
                    Err(e) => IpcResponse::error(format!("{e}")),
                }
            } else {
                IpcResponse::error("RGB controller not initialized")
            }
        }

        IpcRequest::GetLcdTemplates => super::templates::get(state),
        IpcRequest::SetLcdTemplates { templates } => super::templates::set(state, tx, templates),
        IpcRequest::InstallTemplate { template } => super::catalog::install(state, tx, template),
        IpcRequest::RenderTemplatePreview {
            template,
            width,
            height,
        } => super::lcd::render_template_preview(template, width, height),

        IpcRequest::SetLedColor {
            device_id,
            zone,
            led_index,
            color,
        } => super::rgb::set_led_color(state, device_id, zone, led_index, color),
        IpcRequest::GetZoneColors { device_id, zone } => {
            super::rgb::get_zone_colors(state, device_id, zone)
        }

        IpcRequest::SaveRgbPreset { name, device_id } => {
            super::presets::save(state, tx, name, device_id)
        }
        IpcRequest::DeleteRgbPreset { name, device_id } => {
            super::presets::delete(state, tx, name, device_id)
        }
        IpcRequest::ListRgbPresets => super::presets::list(state),
        IpcRequest::ApplyRgbPreset { name, device_id } => {
            super::presets::apply(state, tx, name, device_id)
        }
        IpcRequest::SaveDeviceProfile { name, device_id } => {
            super::profiles::save(state, tx, name, device_id)
        }
        IpcRequest::DeleteDeviceProfile { name } => super::profiles::delete(state, tx, name),
        IpcRequest::ListDeviceProfiles => super::profiles::list(state),
        IpcRequest::ApplyDeviceProfile { name, device_id } => {
            super::profiles::apply(state, tx, name, device_id)
        }
    }
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn write_response(writer: &mut impl Write, response: &IpcResponse) -> anyhow::Result<()> {
    let json = serde_json::to_string(response)?;
    writer.write_all(json.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_pixel_clean_statuses_indexes_canonical_and_all_only() {
        let mut state = DaemonState::new(PathBuf::from("/tmp/dummy.toml"));
        let until = Instant::now() + Duration::from_secs(60);
        state.pixel_clean_states.push(PixelCleanState {
            session_id: 1,
            device_id: Some("hid:1-2:1.0#0".to_string()),
            duration_minutes: 10,
            clean_until: until,
        });
        state.pixel_clean_states.push(PixelCleanState {
            session_id: 2,
            device_id: None,
            duration_minutes: 30,
            clean_until: until,
        });

        let statuses = state.pixel_clean_statuses();
        assert!(statuses.contains_key("hid:1-2:1.0#0"));
        assert!(statuses.contains_key("all"));
        // Bare numeric key "0" should NOT be present
        assert!(!statuses.contains_key("0"));
    }
}
