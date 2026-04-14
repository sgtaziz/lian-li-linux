//! Template editor: callbacks for the `TemplateEditorWindow`, preview
//! rendering over IPC, and the save path back into `lcd_templates.json`.

mod apply;
mod mapping;
mod ops;
mod preview;
mod reflect;

use crate::conversions;
use crate::{EditorRange, EditorWidget, MainWindow, Shared, TemplateEditorWindow};
use lianli_shared::fonts::{cached_system_fonts, DEFAULT_FONT_LABEL};
use lianli_shared::media::SensorRange;
use lianli_shared::screen::screen_presets;
use lianli_shared::template::{LcdTemplate, TemplateBackground, WidgetKind};
use parking_lot::Mutex as PLMutex;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct EditorState {
    pub template: Option<LcdTemplate>,
    pub target_lcd_index: Option<usize>,
    pub selected_widget: i32,
    pub preview_version: u64,
}

pub type SharedEditor = Arc<PLMutex<EditorState>>;

pub struct EditorHandle {
    pub window: TemplateEditorWindow,
    pub state: SharedEditor,
}

pub fn install(main: &MainWindow, shared: Shared) -> EditorHandle {
    let editor = TemplateEditorWindow::new().expect("Failed to create template editor window");
    let editor_state: SharedEditor = Arc::new(PLMutex::new(EditorState::default()));
    let preview_version = Arc::new(AtomicU64::new(0));

    let presets: Vec<SharedString> = screen_presets()
        .iter()
        .map(|p| SharedString::from(p.label))
        .collect();
    editor.set_device_presets(ModelRc::new(VecModel::from(presets)));

    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_sensor_options(conversions::sensor_options_model(&sensors, false));

    let mut font_labels: Vec<SharedString> = vec![SharedString::from(DEFAULT_FONT_LABEL)];
    font_labels.extend(
        cached_system_fonts()
            .iter()
            .map(|f| SharedString::from(f.family.as_str())),
    );
    editor.set_font_names(ModelRc::new(VecModel::from(font_labels)));

    {
        let editor_state = editor_state.clone();
        let shared = shared.clone();
        let editor_weak = editor.as_weak();
        let main_weak = main.as_weak();
        editor.on_save_requested(move || match ops::commit_save(&editor_state, &shared) {
            Ok(()) => {
                if let Some(e) = editor_weak.upgrade() {
                    e.hide().ok();
                }
                crate::refresh_lcd_ui(&main_weak, &shared);
            }
            Err(msg) => {
                if let Some(e) = editor_weak.upgrade() {
                    e.set_status_message(SharedString::from(msg));
                    e.set_status_is_error(true);
                }
            }
        });
    }

    {
        let editor_weak = editor.as_weak();
        editor.on_cancel(move || {
            if let Some(e) = editor_weak.upgrade() {
                e.hide().ok();
            }
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        editor.on_copy_json_requested(move || {
            let json = {
                let st = editor_state.lock();
                match st.template.as_ref() {
                    Some(tpl) => {
                        let mut portable = tpl.clone();
                        ops::portablize_for_export(&mut portable);
                        serde_json::to_string_pretty(&portable).ok()
                    }
                    None => None,
                }
            };
            let Some(e) = editor_weak.upgrade() else {
                return;
            };
            match json {
                Some(text) => match ops::copy_to_clipboard(text) {
                    Ok(()) => {
                        e.set_status_message(SharedString::from(
                            "Template JSON copied to clipboard",
                        ));
                        e.set_status_is_error(false);
                    }
                    Err(err) => {
                        e.set_status_message(SharedString::from(format!("Copy failed: {err}")));
                        e.set_status_is_error(true);
                    }
                },
                None => {
                    e.set_status_message(SharedString::from("No template open to copy"));
                    e.set_status_is_error(true);
                }
            }
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_header_field(move |field, val| {
            let mut needs_widgets_refresh = false;
            {
                let mut st = editor_state.lock();
                let prev_color = reflect::editor_color_from_state(&st);
                let Some(tpl) = st.template.as_mut() else {
                    return;
                };
                match field.as_str() {
                    "name" => tpl.name = val.to_string(),
                    "rotated" => {
                        let new = val.as_str() == "true";
                        if new != tpl.rotated && tpl.base_width != tpl.base_height {
                            ops::apply_rotation_swap(tpl, new);
                            needs_widgets_refresh = true;
                        }
                        tpl.rotated = new;
                    }
                    "bg_type" => {
                        if val.as_str() == "color" {
                            tpl.background = TemplateBackground::Color { rgb: prev_color };
                        }
                    }
                    "bg_r" | "bg_g" | "bg_b" => {
                        let mut rgba = prev_color;
                        rgba[3] = 255;
                        let v = val.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8;
                        match field.as_str() {
                            "bg_r" => rgba[0] = v,
                            "bg_g" => rgba[1] = v,
                            _ => rgba[2] = v,
                        }
                        tpl.background = TemplateBackground::Color { rgb: rgba };
                    }
                    _ => {}
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_header(&e, &editor_state);
                if needs_widgets_refresh {
                    reflect::reflect_widgets_model(&e, &editor_state, &shared);
                }
                e.set_status_message(SharedString::default());
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_preset_selected(move |label| {
            {
                let mut st = editor_state.lock();
                let Some(tpl) = st.template.as_mut() else {
                    return;
                };
                if let Some(preset) = screen_presets().iter().find(|p| p.label == label.as_str()) {
                    tpl.base_width = preset.width;
                    tpl.base_height = preset.height;
                    tpl.target_device = Some(preset.label.to_string());
                    if preset.width == preset.height {
                        tpl.rotated = false;
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_header(&e, &editor_state);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // Drag handler must use in-place row updates; rebuilding the widgets
    // model would tear down the for-loop instances and kill the live drag.
    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_moved(move |idx, x, y| {
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(w) = tpl.widgets.get_mut(idx as usize) {
                        w.x = x;
                        w.y = y;
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widget_row(&e, &editor_state, &shared, idx as usize);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_added(move |kind_label| {
            let kind_id = WidgetKind::kind_id_for_friendly(kind_label.as_str()).unwrap_or("label");
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    let (cx, cy) = (tpl.base_width as f32 / 2.0, tpl.base_height as f32 / 2.0);
                    let id = format!("w-{}", tpl.widgets.len() + 1);
                    tpl.widgets
                        .push(mapping::make_default_widget(&id, kind_id, cx, cy));
                    st.selected_widget = (tpl.widgets.len() - 1) as i32;
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widgets_model(&e, &editor_state, &shared);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_removed(move |idx| {
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    let i = idx as usize;
                    if i < tpl.widgets.len() {
                        tpl.widgets.remove(i);
                        st.selected_widget = -1;
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widgets_model(&e, &editor_state, &shared);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_reorder(move |from, to| {
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    let len = tpl.widgets.len();
                    let (f, t) = (from as usize, to as usize);
                    if f < len && t < len && f != t {
                        let w = tpl.widgets.remove(f);
                        tpl.widgets.insert(t, w);
                        if st.selected_widget == from {
                            st.selected_widget = to;
                        } else if from < to && st.selected_widget > from && st.selected_widget <= to
                        {
                            st.selected_widget -= 1;
                        } else if to < from && st.selected_widget >= to && st.selected_widget < from
                        {
                            st.selected_widget += 1;
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widgets_model(&e, &editor_state, &shared);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_field(move |idx, field, val| {
            if field.as_str() == "_select" {
                editor_state.lock().selected_widget = idx;
                if let Some(e) = editor_weak.upgrade() {
                    reflect::select_widget(&e, &editor_state, &shared, idx);
                }
                return;
            }
            {
                let sensors = shared.lock().unwrap().available_sensors.clone();
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(widget) = tpl.widgets.get_mut(idx as usize) {
                        apply::apply_widget_field(widget, field.as_str(), val.as_str(), &sensors);
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widget_row(&e, &editor_state, &shared, idx as usize);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_pick_bg_image(move || {
            let editor_state2 = editor_state.clone();
            let editor_weak2 = editor_weak.clone();
            let preview_version2 = preview_version.clone();
            let shared2 = shared.clone();
            std::thread::spawn(move || {
                let file = rfd::FileDialog::new()
                    .add_filter("Images", &["jpg", "jpeg", "png", "bmp"])
                    .pick_file();
                if let Some(path) = file {
                    {
                        let mut st = editor_state2.lock();
                        if let Some(tpl) = st.template.as_mut() {
                            tpl.background = TemplateBackground::Image { path };
                        }
                    }
                    let editor_weak3 = editor_weak2.clone();
                    let editor_state3 = editor_state2.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(e) = editor_weak3.upgrade() {
                            reflect::reflect_header(&e, &editor_state3);
                        }
                    })
                    .ok();
                    preview::request_preview(
                        &editor_weak2,
                        &editor_state2,
                        &preview_version2,
                        &shared2,
                    );
                }
            });
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_pick_widget_image(move |idx| {
            let editor_state2 = editor_state.clone();
            let editor_weak2 = editor_weak.clone();
            let preview_version2 = preview_version.clone();
            let shared2 = shared.clone();
            let idx_usize = idx as usize;
            std::thread::spawn(move || {
                let file = rfd::FileDialog::new()
                    .add_filter(
                        "Media",
                        &[
                            "jpg", "jpeg", "png", "bmp", "gif", "mp4", "webm", "mkv", "avi",
                        ],
                    )
                    .pick_file();
                if let Some(path) = file {
                    {
                        let mut st = editor_state2.lock();
                        if let Some(tpl) = st.template.as_mut() {
                            if let Some(w) = tpl.widgets.get_mut(idx_usize) {
                                match &mut w.kind {
                                    WidgetKind::Image { path: p, .. }
                                    | WidgetKind::Video { path: p, .. } => *p = path,
                                    _ => {}
                                }
                            }
                        }
                    }
                    let editor_weak3 = editor_weak2.clone();
                    let editor_state3 = editor_state2.clone();
                    let shared3 = shared2.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(e) = editor_weak3.upgrade() {
                            reflect::reflect_widget_row(&e, &editor_state3, &shared3, idx_usize);
                        }
                    })
                    .ok();
                    preview::request_preview(
                        &editor_weak2,
                        &editor_state2,
                        &preview_version2,
                        &shared2,
                    );
                }
            });
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_range_field(move |range_idx, field, val| {
            let target_idx = editor_state.lock().selected_widget;
            if target_idx < 0 {
                return;
            }
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(widget) = tpl.widgets.get_mut(target_idx as usize) {
                        if let Some(ranges) = apply::widget_ranges_mut(&mut widget.kind) {
                            if let Some(r) = ranges.get_mut(range_idx as usize) {
                                apply::apply_range_field(r, field.as_str(), val.as_str());
                            }
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_range_added(move || {
            let target_idx = editor_state.lock().selected_widget;
            if target_idx < 0 {
                return;
            }
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(widget) = tpl.widgets.get_mut(target_idx as usize) {
                        if let Some(ranges) = apply::widget_ranges_mut(&mut widget.kind) {
                            ranges.push(SensorRange {
                                max: Some(50.0),
                                color: [200, 200, 200],
                                alpha: 255,
                            });
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_ranges(&e, &editor_state);
                reflect::reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_range_removed(move |range_idx| {
            let target_idx = editor_state.lock().selected_widget;
            if target_idx < 0 {
                return;
            }
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(widget) = tpl.widgets.get_mut(target_idx as usize) {
                        if let Some(ranges) = apply::widget_ranges_mut(&mut widget.kind) {
                            let i = range_idx as usize;
                            if i < ranges.len() {
                                ranges.remove(i);
                            }
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect::reflect_ranges(&e, &editor_state);
                reflect::reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            preview::request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    let _ = main;

    EditorHandle {
        window: editor,
        state: editor_state,
    }
}

pub fn open(
    handle: &EditorHandle,
    shared: &Shared,
    lcd_index: usize,
    initial: Option<LcdTemplate>,
) {
    handle
        .window
        .set_widgets(slint::ModelRc::new(VecModel::<EditorWidget>::default()));
    handle
        .window
        .set_selected_ranges(slint::ModelRc::new(VecModel::<EditorRange>::default()));
    handle.window.set_selected_index(-1);
    handle
        .window
        .set_selected_widget(mapping::blank_editor_widget());
    handle.window.set_preview_image(Image::default());
    handle.window.set_status_message(SharedString::default());
    handle.window.set_status_is_error(false);

    let tpl = initial.unwrap_or_else(|| {
        let existing = shared.lock().unwrap().lcd_templates.clone();
        crate::make_blank_template(&existing)
    });
    ops::set_editing(&handle.state, tpl, lcd_index);

    let sensors = shared.lock().unwrap().available_sensors.clone();
    handle
        .window
        .set_sensor_options(conversions::sensor_options_model(&sensors, false));

    reflect::reflect_header(&handle.window, &handle.state);
    reflect::reflect_widgets_model(&handle.window, &handle.state, shared);
    handle.window.show().ok();

    let version = Arc::new(AtomicU64::new(0));
    preview::request_preview(&handle.window.as_weak(), &handle.state, &version, shared);
}
