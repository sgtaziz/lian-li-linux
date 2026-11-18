use crate::command_error::CommandError;
use gio::glib::ToVariant;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::image::Image;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const ICON_LIGHT: &[u8] = include_bytes!("../icons/tray/light.png");
const ICON_LIGHT_WARNING: &[u8] = include_bytes!("../icons/tray/light-warning.png");
const ICON_LIGHT_OFFLINE: &[u8] = include_bytes!("../icons/tray/light-offline.png");
const ICON_DARK: &[u8] = include_bytes!("../icons/tray/dark.png");
const ICON_DARK_WARNING: &[u8] = include_bytes!("../icons/tray/dark-warning.png");
const ICON_DARK_OFFLINE: &[u8] = include_bytes!("../icons/tray/dark-offline.png");
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const QUIT_REQUESTED_EVENT: &str = "app://quit-requested";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
    visible: bool,
}

struct StatusItem {
    app: AppHandle,
    status: TrayStatus,
    icon: Vec<ksni::Icon>,
}

#[derive(Default)]
struct TraySlot(Mutex<Option<(Handle<StatusItem>, TrayStatus)>>);

#[derive(Default)]
struct EditorDraft(AtomicBool);

fn icon_bytes(glyph: TrayGlyph, state: TrayState) -> &'static [u8] {
    match (glyph, state) {
        (TrayGlyph::Light, TrayState::Normal) => ICON_LIGHT,
        (TrayGlyph::Light, TrayState::Warning) => ICON_LIGHT_WARNING,
        (TrayGlyph::Light, TrayState::Offline) => ICON_LIGHT_OFFLINE,
        (TrayGlyph::Dark, TrayState::Normal) => ICON_DARK,
        (TrayGlyph::Dark, TrayState::Warning) => ICON_DARK_WARNING,
        (TrayGlyph::Dark, TrayState::Offline) => ICON_DARK_OFFLINE,
    }
}

fn pixmap(glyph: TrayGlyph, state: TrayState) -> Vec<ksni::Icon> {
    let Ok(image) = Image::from_bytes(icon_bytes(glyph, state)) else {
        return Vec::new();
    };
    let data = image
        .rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
        .collect();
    vec![ksni::Icon {
        width: image.width() as i32,
        height: image.height() as i32,
        data,
    }]
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = crate::main_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit("tray://shown", ());
}

impl ksni::Tray for StatusItem {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "lianli-linux".into()
    }

    fn title(&self) -> String {
        "Lian Li Linux".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.status.tooltip.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let label = |text: &str| StandardItem {
            label: text.replace('_', "__"),
            enabled: false,
            ..Default::default()
        };
        let mut items = vec![label(&self.status.status).into()];
        if let Some(warning) = &self.status.warning {
            items.push(label(warning).into());
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: self.status.open.replace('_', "__"),
                activate: Box::new(|item: &mut Self| show_main_window(&item.app)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: self.status.quit.replace('_', "__"),
                activate: Box::new(|item: &mut Self| {
                    if item.app.get_webview_window("main").is_none() {
                        request_exit(&item.app);
                    } else {
                        let _ = item.app.emit("tray://quit", ());
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn init(app: &AppHandle) {
    app.manage(TraySlot::default());
    app.manage(EditorDraft::default());
}

fn request_exit(app: &AppHandle) {
    let dirty = app
        .try_state::<EditorDraft>()
        .is_some_and(|draft| draft.0.load(Ordering::Acquire));
    match app.get_webview_window("editor") {
        Some(editor) if dirty => {
            let _ = editor.unminimize();
            let _ = editor.show();
            let _ = editor.set_focus();
            let _ = app.emit_to("editor", QUIT_REQUESTED_EVENT, ());
        }
        _ => app.exit(0),
    }
}

pub fn clear_editor_draft(app: &AppHandle) {
    if let Some(draft) = app.try_state::<EditorDraft>() {
        draft.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub fn set_editor_draft(app: AppHandle, window: WebviewWindow, dirty: bool) {
    if window.label() != "editor" {
        return;
    }
    if let Some(draft) = app.try_state::<EditorDraft>() {
        draft.0.store(dirty, Ordering::Release);
    }
}

fn shutdown(slot: &TraySlot) {
    let item = slot.0.lock().ok().and_then(|mut current| current.take());
    if let Some((handle, _)) = item {
        handle.shutdown().wait();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(slot) = app.try_state::<TraySlot>() {
        shutdown(&slot);
    }
}

#[tauri::command]
pub fn tray_available() -> bool {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .ok()
        .and_then(|bus| {
            bus.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&(WATCHER_NAME,).to_variant()),
                gio::glib::VariantTy::new("(b)").ok(),
                gio::DBusCallFlags::NONE,
                1_000,
                gio::Cancellable::NONE,
            )
            .ok()
        })
        .and_then(|reply| reply.child_value(0).get::<bool>())
        .unwrap_or(false)
}

#[tauri::command]
pub fn set_tray_status(app: AppHandle, status: TrayStatus) -> Result<(), CommandError> {
    let Some(slot) = app.try_state::<TraySlot>() else {
        return Ok(());
    };
    if !status.visible {
        shutdown(&slot);
        return Ok(());
    }
    let mut current = slot
        .0
        .lock()
        .map_err(|_| CommandError::from("Tray state is unavailable"))?;
    if current
        .as_ref()
        .is_some_and(|(handle, _)| handle.is_closed())
    {
        *current = None;
    }
    if let Some((handle, shown)) = current.as_mut() {
        if *shown == status {
            return Ok(());
        }
        let icon = (shown.glyph != status.glyph || shown.state != status.state)
            .then(|| pixmap(status.glyph, status.state));
        let next = status.clone();
        let applied = handle.update(move |item| {
            if let Some(icon) = icon {
                item.icon = icon;
            }
            item.status = next;
        });
        if applied.is_some() {
            *shown = status;
            return Ok(());
        }
        *current = None;
    }
    let item = StatusItem {
        app: app.clone(),
        icon: pixmap(status.glyph, status.state),
        status: status.clone(),
    };
    let handle = item
        .spawn()
        .map_err(|error| CommandError::from(error.to_string()))?;
    *current = Some((handle, status));
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle, force: Option<bool>) {
    if force.unwrap_or(false) {
        app.exit(0);
    } else {
        request_exit(&app);
    }
}
