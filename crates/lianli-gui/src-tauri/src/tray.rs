use serde::Deserialize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

const ICON_LIGHT: &[u8] = include_bytes!("../icons/tray/light.png");
const ICON_LIGHT_WARNING: &[u8] = include_bytes!("../icons/tray/light-warning.png");
const ICON_LIGHT_OFFLINE: &[u8] = include_bytes!("../icons/tray/light-offline.png");
const ICON_DARK: &[u8] = include_bytes!("../icons/tray/dark.png");
const ICON_DARK_WARNING: &[u8] = include_bytes!("../icons/tray/dark-warning.png");
const ICON_DARK_OFFLINE: &[u8] = include_bytes!("../icons/tray/dark-offline.png");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayGlyph {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayState {
    Normal,
    Warning,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrayStatus {
    status: String,
    warning: Option<String>,
    tooltip: String,
    open: String,
    quit: String,
    glyph: TrayGlyph,
    state: TrayState,
}

struct Tray {
    icon: TrayIcon,
    menu: Menu<Wry>,
    status: MenuItem<Wry>,
    warning: MenuItem<Wry>,
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    shown: Mutex<Option<TrayStatus>>,
}

fn icon_bytes(glyph: &TrayGlyph, state: TrayState) -> &'static [u8] {
    match (glyph, state) {
        (TrayGlyph::Light, TrayState::Normal) => ICON_LIGHT,
        (TrayGlyph::Light, TrayState::Warning) => ICON_LIGHT_WARNING,
        (TrayGlyph::Light, TrayState::Offline) => ICON_LIGHT_OFFLINE,
        (TrayGlyph::Dark, TrayState::Normal) => ICON_DARK,
        (TrayGlyph::Dark, TrayState::Warning) => ICON_DARK_WARNING,
        (TrayGlyph::Dark, TrayState::Offline) => ICON_DARK_OFFLINE,
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit("tray://shown", ());
}

fn build(app: &AppHandle) -> tauri::Result<Tray> {
    let status = MenuItem::with_id(app, "status", "Lian Li Linux", false, None::<&str>)?;
    let warning = MenuItem::with_id(app, "warning", "", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Lian Li Linux", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &separator, &open, &quit])?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_LIGHT)?)
        .tooltip("Lian Li Linux")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => {
                let _ = app.emit("tray://quit", ());
            }
            _ => {}
        })
        .build(app)?;
    Ok(Tray {
        icon,
        menu,
        status,
        warning,
        open,
        quit,
        shown: Mutex::new(None),
    })
}

/// Creates the tray icon. Missing AppIndicator libraries make the underlying
/// library panic on Linux, so that case is treated like any other failure.
pub fn init(app: &AppHandle) {
    match catch_unwind(AssertUnwindSafe(|| build(app))) {
        Ok(Ok(tray)) => {
            app.manage(tray);
        }
        Ok(Err(error)) => tracing::warn!("Tray icon unavailable: {error}"),
        Err(_) => tracing::warn!("Tray icon unavailable: no AppIndicator library could be loaded"),
    }
}

fn apply(tray: &Tray, status: &TrayStatus, previous: Option<&TrayStatus>) -> tauri::Result<()> {
    tray.status.set_text(&status.status)?;
    tray.open.set_text(&status.open)?;
    tray.quit.set_text(&status.quit)?;
    let had_warning = previous.is_some_and(|previous| previous.warning.is_some());
    match (&status.warning, had_warning) {
        (Some(text), false) => {
            tray.warning.set_text(text)?;
            tray.menu.insert(&tray.warning, 1)?;
        }
        (Some(text), true) => tray.warning.set_text(text)?,
        (None, true) => tray.menu.remove(&tray.warning)?,
        (None, false) => {}
    }
    tray.icon.set_tooltip(Some(&status.tooltip))?;
    if previous
        .is_none_or(|previous| previous.glyph != status.glyph || previous.state != status.state)
    {
        tray.icon.set_icon(Some(Image::from_bytes(icon_bytes(
            &status.glyph,
            status.state,
        ))?))?;
    }
    Ok(())
}

#[tauri::command]
pub fn tray_available(app: AppHandle) -> bool {
    app.try_state::<Tray>().is_some()
}

#[tauri::command]
pub fn set_tray_status(app: AppHandle, status: TrayStatus) -> Result<(), String> {
    let Some(tray) = app.try_state::<Tray>() else {
        return Ok(());
    };
    let mut shown = tray
        .shown
        .lock()
        .map_err(|_| "Tray state is unavailable".to_string())?;
    if shown.as_ref() == Some(&status) {
        return Ok(());
    }
    apply(&tray, &status, shown.as_ref()).map_err(|error| error.to_string())?;
    *shown = Some(status);
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
