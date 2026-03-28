//! IPC server: Unix domain socket for daemon ↔ GUI communication.
//!
//! Protocol: newline-delimited JSON (one request is one response per connection).
//! The GUI polls periodically for telemetry. Config writes go through IPC.

use crate::rgb_controller::RgbController;
use lianli_shared::config::AppConfig;
use lianli_shared::ipc::{DeviceInfo, IpcRequest, IpcResponse, TelemetrySnapshot};
use parking_lot::Mutex;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub devices: Vec<DeviceInfo>,
    pub telemetry: TelemetrySnapshot,
    /// Set by IPC when a config write comes in; main loop checks and clears.
    pub config_reload_pending: bool,
    /// RGB controller, set once devices are opened.
    pub rgb_controller: Option<Arc<Mutex<RgbController>>>,
    /// Device ID pending display mode switch (LCD→Desktop). Handled by service main loop.
    pub pending_display_switch: Option<String>,
    /// MAC address pending wireless device bind. Handled by service main loop.
    pub pending_bind: Option<String>,
}

impl DaemonState {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config: None,
            config_path,
            devices: Vec::new(),
            telemetry: TelemetrySnapshot::default(),
            config_reload_pending: false,
            rgb_controller: None,
            pending_display_switch: None,
            pending_bind: None,
        }
    }
}

/// Starts the IPC server in a background thread.
/// Returns the join handle for cleanup.
pub fn start_ipc_server(
    state: Arc<Mutex<DaemonState>>,
    stop_flag: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_server(state, stop_flag) {
            error!("IPC server error: {e}");
        }
    })
}

fn run_server(state: Arc<Mutex<DaemonState>>, stop_flag: Arc<AtomicBool>) -> anyhow::Result<()> {
    // Clean up stale socket
    let socket_path = Path::new(SOCKET_PATH.as_str());
    if socket_path.exists() {
        fs::remove_file(socket_path)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).ok();
    }

    let listener = UnixListener::bind(socket_path)?;

    // Make socket world-accessible so non-root GUI can connect
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666))?;
    }

    // Non-blocking so we can check stop_flag
    listener.set_nonblocking(true)?;

    info!("IPC server listening on {}", *SOCKET_PATH);

    while !stop_flag.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _addr)) => {
                // Set blocking for this connection
                stream.set_nonblocking(false).ok();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok();

                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, state) {
                        debug!("IPC connection error: {e}");
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No pending connection, sleep briefly
                thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                warn!("IPC accept error: {e}");
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    // Cleanup socket on exit
    fs::remove_file(socket_path).ok();
    info!("IPC server stopped");
    Ok(())
}

fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    state: Arc<Mutex<DaemonState>>,
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
        let response = handle_request(request, &state);
        write_response(&mut writer, &response)?;
    }

    Ok(())
}

fn handle_request(request: IpcRequest, state: &Arc<Mutex<DaemonState>>) -> IpcResponse {
    match request {
        IpcRequest::Ping => IpcResponse::ok(serde_json::json!("pong")),

        IpcRequest::ListDevices => {
            let state = state.lock();
            IpcResponse::ok(&state.devices)
        }

        IpcRequest::GetConfig => {
            let state = state.lock();
            match &state.config {
                Some(config) => IpcResponse::ok(config),
                None => IpcResponse::error("config not loaded yet"),
            }
        }

        IpcRequest::GetTelemetry => {
            let state = state.lock();
            IpcResponse::ok(&state.telemetry)
        }

        IpcRequest::SetConfig { config } => {
            let mut state = state.lock();
            match write_config(&state.config_path, &config) {
                Ok(()) => {
                    state.config = Some(config);
                    state.config_reload_pending = true;
                    info!("Config updated via IPC");
                    IpcResponse::ok(serde_json::json!(null))
                }
                Err(e) => IpcResponse::error(format!("failed to write config: {e}")),
            }
        }

        IpcRequest::SetLcdMedia { device_id, config } => {
            let mut state = state.lock();
            let app_config = state.config.get_or_insert_with(AppConfig::default);
            let found = app_config.lcds.iter_mut().find(|lcd| lcd.device_id() == device_id);
            match found {
                Some(lcd) => {
                    *lcd = config;
                }
                None => {
                    // New LCD entry
                    app_config.lcds.push(config);
                }
            }
            let cfg_clone = app_config.clone();
            match write_config(&state.config_path, &cfg_clone) {
                Ok(()) => {
                    state.config_reload_pending = true;
                    IpcResponse::ok(serde_json::json!(null))
                }
                Err(e) => IpcResponse::error(format!("failed to write config: {e}")),
            }
        }

        IpcRequest::SetFanConfig { config } => {
            let mut state = state.lock();
            let app_config = state.config.get_or_insert_with(AppConfig::default);
            app_config.fans = Some(config);
            let cfg_clone = app_config.clone();
            match write_config(&state.config_path, &cfg_clone) {
                Ok(()) => {
                    state.config_reload_pending = true;
                    IpcResponse::ok(serde_json::json!(null))
                }
                Err(e) => IpcResponse::error(format!("failed to write config: {e}")),
            }
        }

        IpcRequest::SetFanSpeed {
            device_index,
            fan_pwm,
        } => {
            debug!("SetFanSpeed for device {device_index}: {fan_pwm:?}");
            IpcResponse::ok(serde_json::json!(null))
        }

        IpcRequest::GetRgbCapabilities => {
            let state = state.lock();
            if let Some(ref rgb) = state.rgb_controller {
                let caps = rgb.lock().capabilities();
                IpcResponse::ok(&caps)
            } else {
                IpcResponse::ok(serde_json::json!([]))
            }
        }

        IpcRequest::SetRgbEffect {
            device_id,
            zone,
            effect,
        } => {
            let state = state.lock();
            if let Some(ref rgb) = state.rgb_controller {
                match rgb.lock().set_effect(&device_id, zone, &effect) {
                    Ok(()) => IpcResponse::ok(serde_json::json!(null)),
                    Err(e) => IpcResponse::error(format!("RGB effect error: {e}")),
                }
            } else {
                IpcResponse::error("RGB controller not initialized")
            }
        }

        IpcRequest::SetRgbDirect {
            device_id,
            zone,
            colors,
        } => {
            let state = state.lock();
            if let Some(ref rgb) = state.rgb_controller {
                match rgb.lock().set_direct_colors(&device_id, zone, &colors) {
                    Ok(()) => IpcResponse::ok(serde_json::json!(null)),
                    Err(e) => IpcResponse::error(format!("RGB direct error: {e}")),
                }
            } else {
                IpcResponse::error("RGB controller not initialized")
            }
        }

        IpcRequest::SetMbRgbSync { device_id, enabled } => {
            let state = state.lock();
            if let Some(ref rgb) = state.rgb_controller {
                match rgb.lock().set_mb_rgb_sync(&device_id, enabled) {
                    Ok(()) => IpcResponse::ok(serde_json::json!(null)),
                    Err(e) => IpcResponse::error(format!("MB RGB sync error: {e}")),
                }
            } else {
                IpcResponse::error("RGB controller not initialized")
            }
        }

        IpcRequest::SetFanDirection {
            device_id,
            zone,
            swap_lr,
            swap_tb,
        } => {
            let state = state.lock();
            if let Some(ref rgb) = state.rgb_controller {
                match rgb.lock().set_fan_direction(&device_id, zone, swap_lr, swap_tb) {
                    Ok(()) => IpcResponse::ok(serde_json::json!(null)),
                    Err(e) => IpcResponse::error(format!("Fan direction error: {e}")),
                }
            } else {
                IpcResponse::error("RGB controller not initialized")
            }
        }

        IpcRequest::SetRgbConfig { config } => {
            let mut state = state.lock();
            let app_config = state.config.get_or_insert_with(AppConfig::default);
            app_config.rgb = Some(config);
            let cfg_clone = app_config.clone();
            match write_config(&state.config_path, &cfg_clone) {
                Ok(()) => {
                    state.config_reload_pending = true;
                    IpcResponse::ok(serde_json::json!(null))
                }
                Err(e) => IpcResponse::error(format!("failed to write config: {e}")),
            }
        }

        IpcRequest::SwitchDisplayMode { device_id } => {
            let (family, pid) = {
                let state = state.lock();
                match state.devices.iter().find(|d| d.device_id == device_id) {
                    Some(d) => (Some(d.family), d.pid),
                    None => (None, 0),
                }
            };
            match family {
                Some(f) if f.is_desktop_mode() => {
                    // Desktop -> LCD: send switch bytes via HID to CH340
                    if pid == 0 {
                        return IpcResponse::error("device PID not available");
                    }
                    match hidapi::HidApi::new() {
                        Ok(api) => {
                            match lianli_devices::display_switcher::switch_to_lcd_mode(&api, pid) {
                                Ok(()) => IpcResponse::ok(serde_json::json!({
                                    "switched": "to_lcd",
                                    "message": "Device is rebooting into LCD mode. It will appear shortly."
                                })),
                                Err(e) => IpcResponse::error(format!("switch failed: {e}")),
                            }
                        }
                        Err(e) => IpcResponse::error(format!("failed to open HID: {e}")),
                    }
                }
                Some(f) if f.supports_display_mode_switch() => {
                    // LCD -> Desktop: service loop owns the WinUSB transport
                    let mut state = state.lock();
                    state.pending_display_switch = Some(device_id);
                    IpcResponse::ok(serde_json::json!({
                        "switched": "to_desktop",
                        "message": "Device is switching to desktop mode. It will reboot shortly."
                    }))
                }
                Some(_) => IpcResponse::error("device does not support display mode switching"),
                None => IpcResponse::error(format!("device not found: {device_id}")),
            }
        }

        IpcRequest::BindWirelessDevice { mac } => {
            let mut state = state.lock();
            state.pending_bind = Some(mac);
            IpcResponse::ok(serde_json::json!({
                "message": "Bind command queued. Device should appear shortly."
            }))
        }

        IpcRequest::Subscribe => {
            IpcResponse::error("Subscribe not yet implemented; use polling via GetTelemetry")
        }
    }
}


fn write_config(path: &Path, config: &AppConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    fs::write(path, json)?;
    Ok(())
}

fn write_response(writer: &mut impl Write, response: &IpcResponse) -> anyhow::Result<()> {
    let json = serde_json::to_string(response)?;
    writer.write_all(json.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
