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
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, LazyLock};
use std::thread;
use tracing::{debug, error, info, warn};

pub static SOCKET_PATH: LazyLock<String> = LazyLock::new(|| {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    format!("{runtime_dir}/lianli-daemon.sock")
});

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
        }
    }

    pub fn templates_path(&self) -> PathBuf {
        template_store::templates_path_for(&self.config_path)
    }
}

/// Starts the IPC server in a background thread.
/// Returns the join handle for cleanup.
pub fn start_ipc_server(
    state: Arc<Mutex<DaemonState>>,
    stop_flag: Arc<AtomicBool>,
    tx: Sender<DaemonEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_server(state, stop_flag, tx) {
            error!("IPC server error: {e}");
        }
    })
}

fn run_server(
    state: Arc<Mutex<DaemonState>>,
    stop_flag: Arc<AtomicBool>,
    tx: Sender<DaemonEvent>,
) -> anyhow::Result<()> {
    let socket_path = Path::new(SOCKET_PATH.as_str());
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

    info!("IPC server listening on {}", *SOCKET_PATH);

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
