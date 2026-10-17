//! Tauri command handlers + multi-window setup for the Lian Li Linux GUI.

mod app_update;
mod command_error;
mod diagnostic_export;
mod external_link;
mod installation;
mod ipc;
mod managed_import;
mod media_preview;
mod service_operations;
mod session_worker;
mod startup_image_input;

use command_error::CommandError;
use ipc::PollResult;
use lianli_shared::ui_message::UiMessage;
use serde_json::Value;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Whether we are running under a Wayland session.
fn is_wayland() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return true;
    }
    matches!(std::env::var("XDG_SESSION_TYPE").as_deref(), Ok("wayland"))
}

/// Whether we should drop GTK's client-side decorations and let the compositor
/// draw its own title bar.
///
/// Only tiling / wlroots compositors (Hyprland, sway, river, …) render their
/// own server-side title bar. GNOME and KDE on Wayland rely on client-side
/// decorations, so dropping them there would leave the window with no title bar
/// at all. On X11 the window manager always provides decorations, so we keep
/// GTK's request enabled there too.
///
/// Override with `LIANLI_NO_CSD=1` (force compositor decorations) or
/// `LIANLI_FORCE_CSD=1` (force GTK decorations).
fn compositor_draws_titlebar() -> bool {
    if std::env::var_os("LIANLI_NO_CSD").is_some() {
        return true;
    }
    if std::env::var_os("LIANLI_FORCE_CSD").is_some() {
        return false;
    }
    if !is_wayland() {
        return false;
    }
    const SSD_COMPOSITORS: &[&str] = &[
        "Hyprland", "sway", "river", "wayfire", "labwc", "niri", "dwl", "qtile", "cosmic",
    ];
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    desktop
        .split(':')
        .any(|part| SSD_COMPOSITORS.iter().any(|c| c.eq_ignore_ascii_case(part)))
}

/// On compositors that draw their own title bar, disable GTK's client-side
/// decorations so the native title bar is shown instead.
fn apply_platform_decorations(window: &WebviewWindow) {
    if compositor_draws_titlebar() {
        let _ = window.set_decorations(false);
    }
}

/// Proxy a single IPC method to the daemon.
///
/// Returns the daemon's `data` payload on success or the daemon's error
/// message on failure (so the frontend can surface it).
#[tauri::command]
async fn ipc_request(
    method: String,
    params: Value,
    expected_instance: Option<String>,
) -> Result<Value, command_error::CommandError> {
    // Delegate to the blocking socket client on a dedicated thread so the
    // async Tauri runtime is never blocked on socket I/O.
    let result = tauri::async_runtime::spawn_blocking(move || {
        ipc::request_expected(&method, params, expected_instance.as_deref())
    })
    .await
    .map_err(|e| format!("ipc worker join error: {e}"))??;
    Ok(result)
}

/// Return the application version from Cargo.toml (resolved at compile time).
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn installation_health(
) -> Result<lianli_shared::installation::InstallationReport, CommandError> {
    tauri::async_runtime::spawn_blocking(installation::check)
        .await
        .map_err(|error| format!("Installation check failed: {error}"))?
}

#[tauri::command]
async fn diagnostic_preview(
    window: WebviewWindow,
    input: diagnostic_export::Input,
) -> Result<diagnostic_export::Preview, CommandError> {
    if window.label() != "main" {
        return Err(CommandError::ui(
            "Open diagnostic export from the main window",
            UiMessage::new("gui.diagnostics.previewMainWindow"),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        diagnostic_export::preview(input).map_err(CommandError::from)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn diagnostic_save(
    window: WebviewWindow,
    app: tauri::AppHandle,
    id: u64,
) -> Result<bool, CommandError> {
    if window.label() != "main" {
        return Err(CommandError::ui(
            "Save diagnostic exports from the main window",
            UiMessage::new("gui.diagnostics.saveMainWindow"),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        diagnostic_export::save(&app, id).map_err(CommandError::from)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn diagnostic_preview_file(
    window: WebviewWindow,
    app: tauri::AppHandle,
    input: diagnostic_export::Input,
    title: Option<String>,
) -> Result<Option<diagnostic_export::Preview>, CommandError> {
    if window.label() != "main" {
        return Err(CommandError::ui(
            "Choose daemon logs from the main window",
            UiMessage::new("gui.diagnostics.logsMainWindow"),
        ));
    }
    let title = ui_text(title, "Choose saved daemon logs");
    tauri::async_runtime::spawn_blocking(move || {
        diagnostic_export::preview_file(&app, input, &title).map_err(CommandError::from)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn service_operation_status(
) -> Result<lianli_shared::services::ServiceOperationStatus, String> {
    tauri::async_runtime::spawn_blocking(service_operations::status)
        .await
        .map_err(|error| format!("Service progress check failed: {error}"))
}

#[tauri::command]
async fn managed_import_start(
    instance: String,
    lcds: Vec<lianli_shared::config::LcdConfig>,
    templates: Vec<lianli_shared::template::LcdTemplate>,
) -> Result<String, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        managed_import::start(&instance, lcds, templates).map_err(CommandError::from)
    })
    .await
    .map_err(|error| format!("Import submission worker failed: {error}"))?
}

#[tauri::command]
async fn managed_import_status() -> Result<Option<lianli_control::media_import_job::Status>, String>
{
    tauri::async_runtime::spawn_blocking(|| {
        managed_import::status().map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| format!("Import status worker failed: {error}"))?
}

#[tauri::command]
async fn open_external(url: String) -> Result<(), CommandError> {
    tauri::async_runtime::spawn_blocking(move || external_link::open(&url))
        .await
        .map_err(|error| format!("Link worker failed: {error}"))?
}

#[tauri::command]
async fn media_preview(path: String) -> Result<tauri::ipc::Response, CommandError> {
    tauri::async_runtime::spawn_blocking(move || media_preview::read(std::path::Path::new(&path)))
        .await
        .map_err(|error| format!("Preview worker failed: {error}"))?
        .map(tauri::ipc::Response::new)
}

#[tauri::command]
async fn service_action(
    app: tauri::AppHandle,
    request: lianli_shared::services::ServiceActionRequest,
) -> Result<(), CommandError> {
    let operation = service_operations::Operation::begin(app)?;
    tauri::async_runtime::spawn_blocking(move || operation.run(request))
        .await
        .map_err(|error| format!("Service operation worker failed: {error}"))?
}

#[tauri::command]
async fn managed_import_result(
    instance: String,
    id: String,
) -> Result<lianli_control::media_import::PublishedSelection, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        managed_import::result(&instance, &id).map_err(CommandError::from)
    })
    .await
    .map_err(|error| format!("Import result worker failed: {error}"))?
}

#[tauri::command]
async fn service_change(
    app: tauri::AppHandle,
    request: lianli_shared::services::ServiceChangeRequest,
) -> Result<(), CommandError> {
    let operation = service_operations::Operation::begin(app)?;
    tauri::async_runtime::spawn_blocking(move || operation.run_change(request))
        .await
        .map_err(|error| format!("Service switch submission failed: {error}"))?
}

#[tauri::command]
async fn container_setup_proposal(
    user_config: Option<std::path::PathBuf>,
) -> Result<lianli_control::container_deployment::Deployment, String> {
    tauri::async_runtime::spawn_blocking(move || {
        lianli_control::container_bootstrap::proposal(user_config)
            .map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn container_setup(
    app: tauri::AppHandle,
    deployment: lianli_control::container_deployment::Deployment,
) -> Result<(), CommandError> {
    let operation = service_operations::Operation::begin(app)?;
    tauri::async_runtime::spawn_blocking(move || operation.run_setup(deployment))
        .await
        .map_err(|error| error.to_string())?
}

/// Identity, devices and telemetry from the selected daemon.
#[tauri::command]
async fn poll_daemon() -> Result<PollResult, String> {
    let result = tauri::async_runtime::spawn_blocking(ipc::poll)
        .await
        .map_err(|e| format!("poll worker join error: {e}"))?;
    Ok(result)
}

/// Probe daemon liveness and return `(connected, socket_path)`.
#[tauri::command]
async fn connection_info() -> Result<(bool, String), String> {
    let info = tauri::async_runtime::spawn_blocking(ipc::connection_info)
        .await
        .map_err(|e| format!("connection probe join error: {e}"))?;
    Ok(info)
}

/// A system font discovered via `fc-list` (family + file path).
#[derive(serde::Serialize)]
struct SystemFont {
    family: String,
    path: String,
}

/// Enumerate installed fonts. Delegates to `lianli-shared`'s `fc-list` helper
/// (cached after first call) so the frontend can populate font dropdowns.
#[tauri::command]
fn list_system_fonts() -> Vec<SystemFont> {
    lianli_shared::fonts::cached_system_fonts()
        .iter()
        .map(|f| SystemFont {
            family: f.family.clone(),
            path: f.path.display().to_string(),
        })
        .collect()
}

/// Open (or focus) the LCD template editor as a separate webview window.
///
/// If `template_id` is given, the editor loads that specific template
/// (e.g. when invoked from an LCD config's "Edit" button). When the window
/// already exists, it is navigated to the requested template and focused.
#[tauri::command]
async fn open_editor_window(
    app: tauri::AppHandle,
    template_id: Option<String>,
    title: Option<String>,
) -> Result<(), CommandError> {
    if service_operations::active() {
        return Err(CommandError::ui(
            "Wait for the service operation to finish before opening the template editor.",
            UiMessage::new("gui.service.editorBlocked"),
        ));
    }
    let hash = match &template_id {
        Some(id) if !id.is_empty() => format!("#/editor?template={id}"),
        _ => "#/editor".to_string(),
    };
    if let Some(existing) = app.get_webview_window("editor") {
        let _ = existing.eval(format!("window.location.hash = '{hash}';"));
        let _ = existing.set_focus();
        return Ok(());
    }
    let title = ui_text(title, "Template Editor");
    Ok(open_secondary_window(
        &app, "editor", &title, &hash, 1280, 800,
    )?)
}

/// Open (or focus) the template browser as a separate webview window.
#[tauri::command]
async fn open_browser_window(app: tauri::AppHandle, title: Option<String>) -> Result<(), String> {
    let title = ui_text(title, "Template Browser");
    open_secondary_window(&app, "browser", &title, "#/browser", 1000, 720)
}

/// Text the frontend translated, with the English fallback for older callers.
pub(crate) fn ui_text(translated: Option<String>, fallback: &str) -> String {
    translated
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn open_secondary_window(
    app: &tauri::AppHandle,
    label: &str,
    title: &str,
    hash: &str,
    width: u32,
    height: u32,
) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(label) {
        existing
            .set_focus()
            .map_err(|e| format!("focus error: {e}"))?;
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(hash.into()))
        .title(title)
        .inner_size(width as f64, height as f64)
        .min_inner_size(640.0, 420.0)
        .resizable(true);

    builder = builder.initialization_script(format!("window.__LIANLI_WINDOW__ = '{label}';"));

    builder
        .build()
        .inspect(apply_platform_decorations)
        .map(|_| ())
        .map_err(|e| format!("failed to create {label} window: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("lianli_gui=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            app_update::check_package_update,
            app_update::start_package_update,
            startup_image_input::pick_startup_image,
            ipc_request,
            poll_daemon,
            connection_info,
            list_system_fonts,
            open_editor_window,
            open_browser_window,
            app_version,
            installation_health,
            diagnostic_preview,
            diagnostic_save,
            diagnostic_preview_file,
            service_operation_status,
            managed_import_start,
            managed_import_status,
            managed_import_result,
            media_preview,
            open_external,
            service_action,
            service_change,
            container_setup_proposal,
            container_setup,
        ])
        .setup(|app| {
            tauri::async_runtime::spawn_blocking(|| {
                if let Err(error) = lianli_control::automatic_recovery::trigger() {
                    tracing::warn!("Pending service recovery: {error:#}");
                }
            });
            session_worker::start();
            if let Some(win) = app.get_webview_window("main") {
                apply_platform_decorations(&win);
                #[cfg(debug_assertions)]
                win.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
