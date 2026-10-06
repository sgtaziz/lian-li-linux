//! IPC server: Unix domain socket for daemon-GUI communication.
//!
//! Protocol: newline-delimited JSON, one response per request.
//! The GUI polls periodically for telemetry. Config writes go through IPC.

use crate::controllers::rgb::RgbController;
use crate::service::DaemonEvent;
use crate::template_store;
use lianli_shared::config::AppConfig;
use lianli_shared::daemon::{DaemonInfo, DaemonMode, IPC_PROTOCOL_VERSION};
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
    pub startup_image: Option<lianli_shared::startup_image::StartupImageStatus>,
    pub startup_image_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub info: DaemonInfo,
    pub write_gate: Arc<lianli_control::write_gate::ServiceWriteGate>,
    pub config: Option<AppConfig>,
    pub runtime_hid_backend: Option<lianli_shared::config::HidBackend>,
    pub state_health: crate::state_health::StateHealth,
    pub config_path: PathBuf,
    pub presets_path: PathBuf,
    pub devices: Vec<DeviceInfo>,
    pub openrgb_retry_pending: bool,
    pub media_retry_pending: bool,
    pub catalog_install: Arc<Mutex<Option<lianli_shared::template::catalog::CatalogInstallStatus>>>,
    pub catalog_control: Arc<super::catalog::CatalogControl>,
    pub catalog_runtime: Arc<crate::catalog_references::RuntimeReferences>,
    pub catalog_review: Arc<Mutex<Option<lianli_shared::template::catalog::CatalogReviewStatus>>>,
    pub managed_review: Arc<Mutex<Option<lianli_shared::template::catalog::CatalogReviewStatus>>>,
    pub telemetry: TelemetrySnapshot,
    pub wireless_operations: super::wireless::WirelessOperations,
    /// RGB controller, set once devices are opened.
    pub rgb_controller: Option<Arc<Mutex<RgbController>>>,
    pub user_templates: Vec<LcdTemplate>,
    pub rgb_presets: Vec<RgbPreset>,
    pub pixel_clean_states: Vec<PixelCleanState>,
    pub pixel_clean_preparation: Option<(u64, bool, Option<String>)>,
}

pub fn build_info() -> lianli_shared::daemon::DaemonBuildInfo {
    lianli_shared::daemon::DaemonBuildInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: IPC_PROTOCOL_VERSION,
        capabilities: vec![
            lianli_shared::daemon::GUARDED_WRITES.into(),
            lianli_shared::daemon::GRACEFUL_SHUTDOWN.into(),
            lianli_shared::daemon::SERVICE_STOP.into(),
            lianli_shared::daemon::SERVICE_WRITE_GATE.into(),
            lianli_shared::daemon::SERVICE_SELECTION.into(),
            lianli_shared::daemon::SERVICE_STARTUP_GATE.into(),
            lianli_shared::daemon::MEDIA_DECODE.into(),
            lianli_shared::daemon::INSTALLATION_HEALTH.into(),
            lianli_shared::daemon::DESKTOP_RETRY.into(),
            lianli_shared::daemon::SENSOR_PREVIEW.into(),
            "daemon_info".into(),
            "hardware_video".into(),
            "media_preparation".into(),
            "media_access".into(),
            "state_recovery".into(),
            "backup_preview".into(),
            "backup_restore".into(),
            "backup_cleanup".into(),
            "openrgb_retry".into(),
            "media_retry".into(),
            "startup_recovery_clear".into(),
            "catalog_install_status".into(),
            "catalog_storage".into(),
            "managed_media_storage".into(),
            "managed_media_review".into(),
            "managed_media_removal".into(),
            "catalog_cleanup_review".into(),
            "catalog_cleanup_removal".into(),
            "managed_template_merge".into(),
        ],
    }
}

impl DaemonState {
    pub fn new(config_path: PathBuf) -> Self {
        let presets_path = config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("rgb_presets.json");
        let rgb_presets = crate::persistence::read_rgb_presets(&presets_path);
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let build = build_info();
        Self {
            startup_image: None,
            startup_image_cancel: None,
            info: DaemonInfo {
                version: build.version,
                protocol_version: build.protocol_version,
                instance_id: format!("{:x}-{:x}", std::process::id(), started.as_nanos()),
                pid: std::process::id(),
                ownership_lock: None,
                service_invocation: None,
                service_operation_lock: None,
                mode: DaemonMode::User,
                config_path: config_path.clone(),
                capabilities: build.capabilities,
            },
            config: None,
            runtime_hid_backend: None,
            state_health: Default::default(),
            write_gate: Arc::new(lianli_control::write_gate::ServiceWriteGate::new(
                &lianli_shared::installation::InstallationContext::detect(),
            )),
            config_path,
            presets_path,
            devices: Vec::new(),
            openrgb_retry_pending: false,
            media_retry_pending: false,
            catalog_install: Default::default(),
            catalog_control: Default::default(),
            catalog_runtime: Default::default(),
            catalog_review: Default::default(),
            managed_review: Default::default(),
            telemetry: TelemetrySnapshot::default(),
            wireless_operations: Default::default(),
            rgb_controller: None,
            user_templates: Vec::new(),
            rgb_presets,
            pixel_clean_states: Vec::new(),
            pixel_clean_preparation: None,
        }
    }

    pub fn templates_path(&self) -> PathBuf {
        template_store::templates_path_for(&self.config_path)
    }

    pub fn pixel_clean_statuses(&self) -> HashMap<String, PixelCleanStatus> {
        let now = Instant::now();
        self.pixel_clean_states
            .iter()
            .map(|state| {
                let status = PixelCleanStatus {
                    active: true,
                    session_id: Some(state.session_id),
                    device_id: state.device_id.clone(),
                    duration_minutes: state.duration_minutes,
                    remaining_seconds: state.clean_until.saturating_duration_since(now).as_secs(),
                };
                (
                    state.device_id.clone().unwrap_or_else(|| "all".into()),
                    status,
                )
            })
            .collect()
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
    let peer_uid = super::service_stop::peer_uid(&stream)?;
    let write_gate = state.lock().write_gate.clone();
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

        let authorized = request.authorize(&state.lock().info);
        let response = match authorized {
            Ok(IpcRequest::StopService { invocation_id }) => {
                let info = state.lock().info.clone();
                super::service_stop::request(
                    &info,
                    &invocation_id,
                    peer_uid,
                    unsafe { libc::geteuid() },
                    || signal_hook::low_level::raise(signal_hook::consts::SIGTERM),
                )
            }
            Ok(request) => match write_gate.guard(&request) {
                Ok(permit) => {
                    let sender = super::EventSender::new(tx.clone(), permit);
                    let response = handle_request(request, &state, sender.clone());
                    if sender.delivery_failed() && matches!(response, IpcResponse::Ok { .. }) {
                        IpcResponse::error("The daemon stopped before applying the request. Changes may be saved. Reconnect and check settings before retrying.")
                    } else {
                        response
                    }
                }
                Err(error) => IpcResponse::error(format!("{error:#}")),
            },
            Err(message) => IpcResponse::error(message),
        };
        write_response(&mut writer, &response)?;
    }

    Ok(())
}

fn handle_request(
    request: IpcRequest,
    state: &Arc<Mutex<DaemonState>>,
    tx: super::EventSender,
) -> IpcResponse {
    if matches!(
        request,
        IpcRequest::SwitchDisplayMode { .. }
            | IpcRequest::ClearStartupImageRecovery { .. }
            | IpcRequest::RebootWirelessLcd { .. }
            | IpcRequest::SetLcdBrightness { .. }
            | IpcRequest::StartPixelClean { .. }
    ) && state
        .lock()
        .startup_image
        .as_ref()
        .is_some_and(|job| job.status.is_pending())
    {
        return IpcResponse::error(
            "Wait for the startup image upload to finish before changing LCD state",
        );
    }
    match request {
        IpcRequest::Guarded { .. } => IpcResponse::error("Request was not authorized"),
        IpcRequest::StopService { .. } => {
            IpcResponse::error("Service stop requires authenticated peer credentials")
        }
        IpcRequest::Ping => super::system::ping(),
        IpcRequest::RetryOpenRgb => super::system::retry_openrgb(state, tx),
        IpcRequest::RetryMedia => super::system::retry_media(state, tx),
        IpcRequest::ClearStartupImageRecovery { device_id } => {
            super::system::clear_startup_recovery(state, tx, device_id)
        }
        IpcRequest::GetDaemonInfo => super::system::daemon_info(state),
        IpcRequest::ListStateBackups => {
            super::backups::run(state, super::backups::Operation::List, tx)
        }
        IpcRequest::PreviewStateBackup { target, preserved } => super::backups::run(
            state,
            super::backups::Operation::Preview { target, preserved },
            tx,
        ),
        IpcRequest::DeleteStateBackup {
            target,
            preserved,
            sha256,
        } => super::backups::run(
            state,
            super::backups::Operation::Delete {
                target,
                preserved,
                sha256,
            },
            tx,
        ),
        IpcRequest::RestoreStateBackup { target, sha256 } => super::backups::run(
            state,
            super::backups::Operation::Restore { target, sha256 },
            tx,
        ),
        IpcRequest::GetInstallationHealth => super::installation::check(state),
        IpcRequest::ListSensors => super::system::list_sensors(state),
        IpcRequest::ListPwmHeaders => super::system::list_pwm_headers(),
        IpcRequest::ListDevices => super::system::list_devices(state),
        IpcRequest::GetConfig => super::system::get_config(state),
        IpcRequest::CheckMediaAccess { lcds, templates } => {
            let config_directory = state
                .lock()
                .config_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            match crate::media_access::check(crate::media_access::Input {
                lcds,
                templates,
                config_directory,
            }) {
                Ok(report) => IpcResponse::ok(report),
                Err(error) => IpcResponse::error(format!("{error:#}")),
            }
        }
        IpcRequest::GetTelemetry => super::system::get_telemetry(state),

        IpcRequest::SetConfig { config } => {
            if let Some(error) = super::fan::validate_quantities(&config, &state.lock().devices) {
                return IpcResponse::error(error);
            }
            if let Some(rgb_config) = &config.rgb {
                if let Some(response) = super::rgb::validate_saved_config(state, rgb_config) {
                    return response;
                }
            }
            let mut state = state.lock();
            if let Some(error) =
                super::config::validate_aio_modes(&config, state.config.as_ref(), &state.devices)
            {
                return IpcResponse::error(error);
            }
            super::persist_and_notify(&mut state, &tx, "SetConfig", *config)
        }

        IpcRequest::SetLcdMedia { device_id, config } => {
            super::config::set_lcd_media(state, tx, device_id, *config)
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
        IpcRequest::RetryDesktopDisplay {
            bus,
            address,
            product_id,
        } => super::lcd::retry_desktop(state, &tx, bus, address, product_id),

        IpcRequest::GetWirelessOperation { operation_id } => {
            super::wireless::operation(state, operation_id)
        }
        IpcRequest::BindWirelessDevice { mac } => super::wireless::bind(state, tx, mac),
        IpcRequest::UnbindWirelessDevice { mac } => super::wireless::unbind(state, tx, mac),
        IpcRequest::RebootWirelessLcd { device_id } => super::wireless::reboot_lcd(tx, device_id),
        IpcRequest::DisableLc217Wifi { device_id, disable } => {
            super::wireless::disable_lc217_wifi(tx, device_id, disable)
        }
        IpcRequest::BindAllWireless => super::wireless::bind_all(tx),
        IpcRequest::UnbindAllWireless => super::wireless::unbind_all(tx),
        IpcRequest::GetChannel => super::wireless::get_channel(state),
        IpcRequest::SetMergeLightingConfig { config } => {
            let mut rgb = state
                .lock()
                .config
                .as_ref()
                .and_then(|c| c.rgb.clone())
                .unwrap_or_default();
            rgb.merge_lighting = Some(config.clone());
            if let Some(error) = super::rgb::validate_config(state, &rgb) {
                return error;
            }
            let mut state = state.lock();
            if let Some(mut app_config) = state.config.clone() {
                app_config
                    .rgb
                    .get_or_insert_with(Default::default)
                    .merge_lighting = Some(config);
                super::persist_and_notify(&mut state, &tx, "SetMergeLightingConfig", app_config)
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
            request_id,
        } => {
            if brightness > 100 {
                return IpcResponse::error("LCD brightness must be between 0 and 100");
            }
            if request_id.as_ref().is_some_and(|id| id.len() > 128) {
                return IpcResponse::error("LCD brightness request ID is too long");
            }
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            if tx
                .send(DaemonEvent::SetLcdBrightness {
                    device_id,
                    brightness,
                    request_id,
                    deadline: Instant::now() + Duration::from_secs(3),
                    reply: reply_tx,
                })
                .is_err()
            {
                return IpcResponse::error("daemon service not running");
            }
            match reply_rx.recv_timeout(Duration::from_secs(4)) {
                Ok(Ok(applied)) => IpcResponse::ok(serde_json::json!({
                    "accepted": true, "applied": applied
                })),
                Ok(Err(error)) => IpcResponse::error(error),
                Err(error) => {
                    IpcResponse::error(format!("LCD brightness delivery not confirmed: {error}"))
                }
            }
        }
        IpcRequest::StartPixelClean {
            device_id,
            duration_minutes,
            preparation_id,
        } => {
            if duration_minutes == 0 {
                return IpcResponse::error("Duration must be positive");
            }
            let duration_minutes = duration_minutes.clamp(1, lianli_shared::ipc::MAX_CLEAN_MINUTES);
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            if tx
                .send(DaemonEvent::StartPixelClean {
                    device_id,
                    duration_minutes,
                    preparation_id,
                    deadline: Instant::now() + Duration::from_secs(3),
                    reply: reply_tx,
                })
                .is_err()
            {
                return IpcResponse::error("daemon service not running");
            }
            match reply_rx.recv_timeout(Duration::from_secs(4)) {
                Ok(Ok((session_id, started))) => IpcResponse::ok(
                    serde_json::json!({ "started": started, "session_id": session_id }),
                ),
                Ok(Err(err)) => IpcResponse::error(err),
                Err(e) => IpcResponse::error(format!("timeout starting pixel cleaner: {e}")),
            }
        }
        IpcRequest::StopPixelClean {
            device_id,
            session_id,
        } => {
            if session_id.is_none() {
                return IpcResponse::error("session_id is required");
            }
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
        IpcRequest::GetPixelCleanPreparation { session_id } => {
            match &state.lock().pixel_clean_preparation {
                Some((id, ready, error)) if *id == session_id => {
                    IpcResponse::ok(serde_json::json!({ "ready": ready, "error": error }))
                }
                _ => IpcResponse::error("Unknown or expired pixel cleaner preparation"),
            }
        }
        IpcRequest::UploadStartupImage {
            device_id,
            jpeg_base64,
        } => super::lcd::upload_startup_image(state, &tx, device_id, jpeg_base64),
        IpcRequest::GetStartupImageStatus => {
            IpcResponse::ok(serde_json::json!(state.lock().startup_image))
        }
        IpcRequest::CancelStartupImage { id } => super::lcd::cancel_startup_image(state, id),
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
        IpcRequest::MergeLcdTemplates { originals, copies } => {
            super::templates::merge(state, tx, originals, copies)
        }
        IpcRequest::InstallTemplate { template } => super::catalog::install(state, tx, template),
        IpcRequest::StartCatalogInstall { template } => super::catalog::start(state, tx, template),
        IpcRequest::GetCatalogInstallStatus => super::catalog::status(state),
        IpcRequest::GetCatalogStorage => super::catalog::storage(state, false),
        IpcRequest::GetManagedMediaStorage => super::catalog::storage(state, true),
        IpcRequest::StartCatalogReview { directory } => {
            super::catalog_cleanup::start(state, directory, false)
        }
        IpcRequest::GetCatalogReview { operation_id } => {
            super::catalog_cleanup::status(state, &operation_id, false)
        }
        IpcRequest::StartManagedMediaReview { directory } => {
            super::catalog_cleanup::start(state, directory, true)
        }
        IpcRequest::GetManagedMediaReview { operation_id } => {
            super::catalog_cleanup::status(state, &operation_id, true)
        }
        IpcRequest::StartCatalogRemoval { operation_id } => {
            super::catalog_cleanup::remove(state, &operation_id, tx, false)
        }
        IpcRequest::StartManagedMediaRemoval { operation_id } => {
            super::catalog_cleanup::remove(state, &operation_id, tx, true)
        }
        IpcRequest::RenderTemplatePreview {
            template,
            width,
            height,
        } => {
            let catalog_runtime = state.lock().catalog_runtime.clone();
            let hardware_video = state
                .lock()
                .config
                .as_ref()
                .is_some_and(|config| config.hardware_video);
            super::lcd::render_template_preview(
                template,
                width,
                height,
                hardware_video,
                &catalog_runtime,
            )
        }
        IpcRequest::RenderSensorPreview { lcd, width, height } => {
            super::lcd::render_sensor_preview(state, *lcd, width, height)
        }

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
    fn socket_serializes_service_operations_with_authorized_config_writes() {
        use std::os::fd::AsRawFd;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let mut daemon = DaemonState::new(path.clone());
        let control = root.path().join("control.lock");
        fs::write(&control, "").unwrap();
        daemon.write_gate = Arc::new(lianli_control::write_gate::ServiceWriteGate::from_path(
            control.clone(),
            false,
        ));
        let operation = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(control)
            .unwrap();
        let reserve =
            || unsafe { libc::flock(operation.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(reserve(), 0);
        daemon.config = Some(AppConfig::default());
        let info = daemon.info.clone();
        let state = Arc::new(Mutex::new(daemon));
        let (tx, rx) = std::sync::mpsc::channel();
        let exchange = |request: IpcRequest| {
            let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            server
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let state = state.clone();
            let tx = tx.clone();
            let worker = thread::spawn(move || handle_connection(server, state, tx));
            writeln!(client, "{}", serde_json::to_string(&request).unwrap()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            let mut response = String::new();
            BufReader::new(client).read_line(&mut response).unwrap();
            worker.join().unwrap().unwrap();
            serde_json::from_str::<IpcResponse>(&response).unwrap()
        };
        let write = IpcRequest::SetConfig {
            config: Box::new(AppConfig {
                hardware_video: true,
                ..Default::default()
            }),
        };
        assert!(matches!(
            exchange(IpcRequest::GetDaemonInfo),
            IpcResponse::Ok { .. }
        ));
        assert!(matches!(exchange(write.clone()), IpcResponse::Error { .. }));
        let mut guard = info.write_guard(env!("CARGO_PKG_VERSION")).unwrap();
        guard.instance_id = "previous-instance".into();
        assert!(matches!(
            exchange(IpcRequest::Guarded {
                guard,
                request: Box::new(write.clone()),
            }),
            IpcResponse::Error { .. }
        ));
        assert!(!path.exists());
        assert!(!state.lock().config.as_ref().unwrap().hardware_video);
        assert!(rx.try_recv().is_err());
        assert!(matches!(
            exchange(IpcRequest::Guarded {
                guard: info.write_guard(env!("CARGO_PKG_VERSION")).unwrap(),
                request: Box::new(write.clone()),
            }),
            IpcResponse::Error { .. }
        ));
        assert!(!path.exists());
        assert_eq!(
            unsafe { libc::flock(operation.as_raw_fd(), libc::LOCK_UN) },
            0
        );
        assert!(matches!(
            exchange(IpcRequest::Guarded {
                guard: info.write_guard(env!("CARGO_PKG_VERSION")).unwrap(),
                request: Box::new(write),
            }),
            IpcResponse::Ok { .. }
        ));
        assert!(path.is_file());
        assert!(state.lock().config.as_ref().unwrap().hardware_video);
        assert_ne!(reserve(), 0);
        let DaemonEvent::Coordinated { event, permit } = rx.try_recv().unwrap() else {
            panic!("write permit was not queued")
        };
        assert!(matches!(*event, DaemonEvent::IpcUpdate));
        assert_ne!(reserve(), 0);
        drop(permit);
        assert_eq!(reserve(), 0);
        assert!(matches!(
            exchange(IpcRequest::GetConfig),
            IpcResponse::Ok { .. }
        ));
        assert_eq!(
            unsafe { libc::flock(operation.as_raw_fd(), libc::LOCK_UN) },
            0
        );
        drop(rx);
        assert!(matches!(
            exchange(IpcRequest::Guarded {
                guard: info.write_guard(env!("CARGO_PKG_VERSION")).unwrap(),
                request: Box::new(IpcRequest::SetConfig {
                    config: Box::default()
                }),
            }),
            IpcResponse::Error { .. }
        ));
        assert!(!state.lock().config.as_ref().unwrap().hardware_video);
    }

    #[test]
    fn brightness_response_uses_service_delivery_result() {
        for result in [Ok(true), Ok(false), Err("LCD not found".to_owned())] {
            let root = tempfile::tempdir().unwrap();
            let state = Arc::new(Mutex::new(DaemonState::new(
                root.path().join("config.json"),
            )));
            let (tx, rx) = std::sync::mpsc::channel();
            let expected = result.clone();
            let worker = thread::spawn(move || {
                let DaemonEvent::SetLcdBrightness {
                    brightness,
                    request_id,
                    deadline,
                    reply,
                    ..
                } = rx.recv().unwrap()
                else {
                    panic!("expected brightness request");
                };
                assert_eq!(brightness, 30);
                assert_eq!(request_id.as_deref(), Some("test-brightness"));
                assert!(deadline > Instant::now());
                reply.send(result).unwrap();
            });
            let response = handle_request(
                IpcRequest::SetLcdBrightness {
                    device_id: "lcd".into(),
                    brightness: 30,
                    request_id: Some("test-brightness".into()),
                },
                &state,
                tx.into(),
            );
            match (expected, response) {
                (Ok(applied), IpcResponse::Ok { data }) => {
                    assert_eq!(
                        data,
                        serde_json::json!({"accepted": true, "applied": applied})
                    );
                }
                (Err(expected), IpcResponse::Error { message }) => assert_eq!(message, expected),
                result => panic!("unexpected brightness response: {result:?}"),
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn brightness_rejects_invalid_values_and_a_stopped_service() {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(DaemonState::new(
            root.path().join("config.json"),
        )));
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(matches!(
            handle_request(
                IpcRequest::SetLcdBrightness {
                    device_id: "lcd".into(),
                    brightness: 101,
                    request_id: None,
                },
                &state,
                tx.clone().into()
            ),
            IpcResponse::Error { .. }
        ));
        assert!(rx.try_recv().is_err());
        assert!(matches!(
            handle_request(
                IpcRequest::SetLcdBrightness {
                    device_id: "lcd".into(),
                    brightness: 30,
                    request_id: Some("x".repeat(129)),
                },
                &state,
                tx.clone().into()
            ),
            IpcResponse::Error { .. }
        ));
        assert!(rx.try_recv().is_err());
        drop(rx);
        assert!(matches!(
            handle_request(
                IpcRequest::SetLcdBrightness {
                    device_id: "lcd".into(),
                    brightness: 30,
                    request_id: None,
                },
                &state,
                tx.into()
            ),
            IpcResponse::Error { .. }
        ));
    }

    #[test]
    fn failed_config_save_preserves_gui_state_and_does_not_queue_application() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let mut daemon = DaemonState::new(path.clone());
        daemon.config = Some(AppConfig::default());
        let state = Arc::new(Mutex::new(daemon));
        let (tx, rx) = std::sync::mpsc::channel();
        std::fs::create_dir(&path).unwrap();
        let requested = AppConfig {
            hardware_video: true,
            ..Default::default()
        };
        let response = handle_request(
            IpcRequest::SetConfig {
                config: Box::new(requested.clone()),
            },
            &state,
            tx.clone().into(),
        );
        assert!(matches!(response, IpcResponse::Error { .. }));
        assert!(!state.lock().config.as_ref().unwrap().hardware_video);
        assert!(rx.try_recv().is_err());
        std::fs::remove_dir(&path).unwrap();
        let response = handle_request(
            IpcRequest::SetConfig {
                config: Box::new(requested),
            },
            &state,
            tx.into(),
        );
        assert!(matches!(response, IpcResponse::Ok { .. }));
        assert!(state.lock().config.as_ref().unwrap().hardware_video);
        assert!(matches!(rx.try_recv(), Ok(DaemonEvent::IpcUpdate)));
    }

    #[test]
    fn identity_reports_launch_scope_without_changing_ping_or_queueing_work() {
        let root = tempfile::tempdir().unwrap();
        let mut daemon = DaemonState::new("/custom/config.json".into());
        daemon.write_gate = Arc::new(lianli_control::write_gate::ServiceWriteGate::from_path(
            root.path().join("missing"),
            false,
        ));
        daemon.info.mode = DaemonMode::System;
        let expected = daemon.info.clone();
        let state = Arc::new(Mutex::new(daemon));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let IpcResponse::Ok { data } =
                handle_request(IpcRequest::GetDaemonInfo, &state, tx.clone().into())
            else {
                panic!("identity request failed");
            };
            assert_eq!(
                serde_json::from_value::<DaemonInfo>(data).unwrap(),
                expected
            );
        }
        let IpcResponse::Ok { data } = handle_request(IpcRequest::Ping, &state, tx.into()) else {
            panic!("ping failed");
        };
        assert_eq!(data, "pong");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn invalid_cleaner_requests_never_reach_the_service() {
        let state = Arc::new(Mutex::new(DaemonState::new("/tmp/unused-config".into())));
        let (tx, rx) = std::sync::mpsc::channel();
        for request in [
            IpcRequest::StartPixelClean {
                device_id: None,
                duration_minutes: 0,
                preparation_id: None,
            },
            IpcRequest::StopPixelClean {
                device_id: None,
                session_id: None,
            },
        ] {
            assert!(matches!(
                handle_request(request, &state, tx.clone().into()),
                IpcResponse::Error { .. }
            ));
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn elapsed_cleaner_stays_active_until_the_service_restores_it() {
        let mut state = DaemonState::new("/tmp/unused-config".into());
        state.pixel_clean_states.push(PixelCleanState {
            session_id: 7,
            device_id: Some("lcd#0".into()),
            duration_minutes: 1,
            clean_until: Instant::now() - Duration::from_secs(1),
        });
        let statuses = state.pixel_clean_statuses();
        assert!(statuses["lcd#0"].active);
        assert_eq!(statuses["lcd#0"].remaining_seconds, 0);
        state.pixel_clean_states.clear();
        assert!(state.pixel_clean_statuses().is_empty());
    }

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
