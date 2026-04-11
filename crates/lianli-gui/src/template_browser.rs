//! Template browser: fetches the online catalog manifest and installs
//! selected templates. Session-only state — closing the window drops
//! everything in memory; only [`install`]'d templates persist (to
//! `~/.config/lianli/templates/<id>/` via `lianli_shared::template_catalog`).

use crate::{refresh_lcd_ui, CatalogEntry, MainWindow, Shared, TemplateBrowserWindow};
use lianli_shared::template_catalog::{self, CatalogManifest, CatalogTemplate};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const DAEMON_VERSION_STR: &str = env!("CARGO_PKG_VERSION");

pub struct BrowserHandle {
    pub window: TemplateBrowserWindow,
    pub catalog: Arc<Mutex<Vec<CatalogTemplate>>>,
}

pub fn install(main: &MainWindow, shared: Shared) -> BrowserHandle {
    let window = TemplateBrowserWindow::new().expect("create TemplateBrowserWindow");

    let catalog: Arc<Mutex<Vec<CatalogTemplate>>> = Arc::new(Mutex::new(Vec::new()));

    {
        let weak = window.as_weak();
        let catalog = catalog.clone();
        window.on_refresh_requested(move || {
            let weak = weak.clone();
            let catalog = catalog.clone();
            start_fetch(weak, catalog);
        });
    }

    {
        window.on_publishing_guide_requested(|| {
            const URL: &str = "https://github.com/sgtaziz/lian-li-linux/tree/main/templates";
            if let Err(e) = std::process::Command::new("xdg-open")
                .arg(URL)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                tracing::warn!("xdg-open failed for {URL}: {e}");
            }
        });
    }

    {
        let weak = window.as_weak();
        let main_weak = main.as_weak();
        let shared = shared.clone();
        let catalog = catalog.clone();
        window.on_install_requested(move |id| {
            let Some(tpl) = catalog
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.id == id.as_str())
                .cloned()
            else {
                return;
            };
            let weak = weak.clone();
            let main_weak = main_weak.clone();
            let shared = shared.clone();
            set_entry_state(&weak, &tpl.id, "installing", "");
            std::thread::spawn(move || {
                let sensors = shared.lock().unwrap().available_sensors.clone();
                match template_catalog::install_template(&tpl, &sensors) {
                    Ok(new_template) => {
                        let new_template = Arc::new(new_template);
                        let id_for_ui = tpl.id.clone();
                        let shared_for_ui = shared.clone();
                        let main_weak_for_ui = main_weak.clone();
                        let weak_for_ui = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            persist_installed(&shared_for_ui, (*new_template).clone());
                            refresh_lcd_ui(&main_weak_for_ui, &shared_for_ui);
                            set_entry_state_local(&weak_for_ui, &id_for_ui, "installed", "");
                        });
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        let id_for_ui = tpl.id.clone();
                        let weak_for_ui = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            set_entry_state_local(&weak_for_ui, &id_for_ui, "error", &msg);
                        });
                    }
                }
            });
        });
    }

    BrowserHandle { window, catalog }
}

pub fn open(handle: &BrowserHandle, _shared: &Shared) {
    let window = handle.window.clone_strong();
    window.set_templates(ModelRc::new(VecModel::<CatalogEntry>::default()));
    window.set_error_message(SharedString::default());
    window.set_loading(true);
    window.show().ok();

    let weak = window.as_weak();
    start_fetch(weak, handle.catalog.clone());
}

fn start_fetch(
    weak: slint::Weak<TemplateBrowserWindow>,
    catalog: Arc<Mutex<Vec<CatalogTemplate>>>,
) {
    if let Some(w) = weak.upgrade() {
        w.set_loading(true);
        w.set_error_message(SharedString::default());
        w.set_templates(ModelRc::new(VecModel::<CatalogEntry>::default()));
    }
    let weak = weak.clone();
    std::thread::spawn(move || match template_catalog::fetch_manifest() {
        Ok(manifest) => on_manifest(weak, catalog, manifest),
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    w.set_loading(false);
                    w.set_error_message(SharedString::from(msg));
                }
            });
        }
    });
}

fn on_manifest(
    weak: slint::Weak<TemplateBrowserWindow>,
    catalog: Arc<Mutex<Vec<CatalogTemplate>>>,
    manifest: CatalogManifest,
) {
    let supported = template_catalog::filter_supported(manifest.templates, DAEMON_VERSION_STR);
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            let entries: Vec<CatalogEntry> = supported
                .iter()
                .map(|t| CatalogEntry {
                    id: SharedString::from(t.id.as_str()),
                    name: SharedString::from(t.name.as_str()),
                    description: SharedString::from(t.description.as_str()),
                    author: SharedString::from(t.author.as_str()),
                    preview: Image::default(),
                    preview_loaded: false,
                    install_state: SharedString::from("idle"),
                    install_error: SharedString::default(),
                    base_width: t.base_width as i32,
                    base_height: t.base_height as i32,
                    rotated: t.rotated,
                })
                .collect();
            *catalog.lock().unwrap() = supported.clone();
            let model: Rc<VecModel<CatalogEntry>> = Rc::new(VecModel::from(entries));
            w.set_templates(ModelRc::from(model));
            w.set_loading(false);
            kick_preview_fetches(w.as_weak(), supported);
        }
    });
}

fn kick_preview_fetches(weak: slint::Weak<TemplateBrowserWindow>, templates: Vec<CatalogTemplate>) {
    for (idx, t) in templates.into_iter().enumerate() {
        let weak = weak.clone();
        std::thread::spawn(move || match template_catalog::fetch_preview(&t) {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w_px, h_px) = (rgba.width(), rgba.height());
                    let raw: Vec<u8> = rgba.into_raw();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = weak.upgrade() {
                            let buf = SharedPixelBuffer::clone_from_slice(&raw, w_px, h_px);
                            let image = Image::from_rgba8(buf);
                            update_entry_preview(&win, idx, image);
                        }
                    });
                }
                Err(e) => tracing::warn!("decoding preview for '{}' failed: {e}", t.id),
            },
            Err(e) => tracing::warn!("fetching preview for '{}' failed: {e:#}", t.id),
        });
    }
}

fn update_entry_preview(win: &TemplateBrowserWindow, idx: usize, image: Image) {
    let model = win.get_templates();
    if let Some(mut entry) = model.row_data(idx) {
        entry.preview = image;
        entry.preview_loaded = true;
        model.set_row_data(idx, entry);
    }
}

fn set_entry_state(weak: &slint::Weak<TemplateBrowserWindow>, id: &str, state: &str, err: &str) {
    let weak = weak.clone();
    let id = id.to_string();
    let state = state.to_string();
    let err = err.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            set_entry_state_local(&w.as_weak(), &id, &state, &err);
        }
    });
}

fn set_entry_state_local(
    weak: &slint::Weak<TemplateBrowserWindow>,
    id: &str,
    state: &str,
    err: &str,
) {
    let Some(win) = weak.upgrade() else {
        return;
    };
    let model = win.get_templates();
    for i in 0..model.row_count() {
        if let Some(mut entry) = model.row_data(i) {
            if entry.id.as_str() == id {
                entry.install_state = SharedString::from(state);
                entry.install_error = SharedString::from(err);
                model.set_row_data(i, entry);
                break;
            }
        }
    }
}

fn persist_installed(shared: &Shared, mut tpl: lianli_shared::template::LcdTemplate) {
    let user_list = {
        let mut state = shared.lock().unwrap();
        tpl.name = crate::next_unique_downloaded_name(&tpl.name, &state.lcd_templates);
        tpl.id = crate::generate_template_id("downloaded");
        state.lcd_templates.push(tpl);
        crate::user_templates_only(&state.lcd_templates)
    };
    crate::send_set_templates(user_list);
}
