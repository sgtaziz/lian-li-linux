//! Template editor: callbacks for the `TemplateEditorWindow`, preview
//! rendering over IPC, and the save path back into `lcd_templates.json`.

use crate::conversions;
use crate::ipc_client;
use crate::{EditorRange, EditorWidget, MainWindow, Shared, TemplateEditorWindow};
use lianli_shared::fonts::{
    cached_system_fonts, font_label_for_path, font_path_for_label, DEFAULT_FONT_LABEL,
};
use lianli_shared::ipc::IpcRequest;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::screen::{screen_preset_label, screen_presets};
use lianli_shared::sensors::{SensorInfo, SensorSource};
use lianli_shared::template::{
    BarOrientation, FontRef, ImageFit, LcdTemplate, TemplateBackground, TextAlign, Widget,
    WidgetKind,
};
use parking_lot::Mutex as PLMutex;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::sync::atomic::{AtomicU64, Ordering};
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
        editor.on_save_requested(move || match commit_save(&editor_state, &shared) {
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
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_header_field(move |field, val| {
            let mut needs_widgets_refresh = false;
            {
                let mut st = editor_state.lock();
                let prev_color = editor_color_from_state(&st);
                let Some(tpl) = st.template.as_mut() else {
                    return;
                };
                match field.as_str() {
                    "name" => tpl.name = val.to_string(),
                    "rotated" => {
                        let new = val.as_str() == "true";
                        if new != tpl.rotated && tpl.base_width != tpl.base_height {
                            apply_rotation_swap(tpl, new);
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
                reflect_header(&e, &editor_state);
                if needs_widgets_refresh {
                    reflect_widgets_model(&e, &editor_state, &shared);
                }
                e.set_status_message(SharedString::default());
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                reflect_header(&e, &editor_state);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                reflect_widget_row(&e, &editor_state, &shared, idx as usize);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                    tpl.widgets.push(make_default_widget(&id, kind_id, cx, cy));
                    st.selected_widget = (tpl.widgets.len() - 1) as i32;
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect_widgets_model(&e, &editor_state, &shared);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                reflect_widgets_model(&e, &editor_state, &shared);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                reflect_widgets_model(&e, &editor_state, &shared);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                    select_widget(&e, &editor_state, &shared, idx);
                }
                return;
            }
            {
                let sensors = shared.lock().unwrap().available_sensors.clone();
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    if let Some(widget) = tpl.widgets.get_mut(idx as usize) {
                        apply_widget_field(widget, field.as_str(), val.as_str(), &sensors);
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect_widget_row(&e, &editor_state, &shared, idx as usize);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                            reflect_header(&e, &editor_state3);
                        }
                    })
                    .ok();
                    request_preview(&editor_weak2, &editor_state2, &preview_version2, &shared2);
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
                            reflect_widget_row(&e, &editor_state3, &shared3, idx_usize);
                        }
                    })
                    .ok();
                    request_preview(&editor_weak2, &editor_state2, &preview_version2, &shared2);
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
                        if let Some(ranges) = widget_ranges_mut(&mut widget.kind) {
                            if let Some(r) = ranges.get_mut(range_idx as usize) {
                                apply_range_field(r, field.as_str(), val.as_str());
                            }
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect_ranges(&e, &editor_state);
                reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                        if let Some(ranges) = widget_ranges_mut(&mut widget.kind) {
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
                reflect_ranges(&e, &editor_state);
                reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
                        if let Some(ranges) = widget_ranges_mut(&mut widget.kind) {
                            let i = range_idx as usize;
                            if i < ranges.len() {
                                ranges.remove(i);
                            }
                        }
                    }
                }
            }
            if let Some(e) = editor_weak.upgrade() {
                reflect_ranges(&e, &editor_state);
                reflect_widget_row(&e, &editor_state, &shared, target_idx as usize);
            }
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
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
    handle.window.set_selected_widget(blank_editor_widget());
    handle.window.set_preview_image(Image::default());
    handle.window.set_status_message(SharedString::default());
    handle.window.set_status_is_error(false);

    let tpl = initial.unwrap_or_else(crate::make_blank_template);
    set_editing(&handle.state, tpl, lcd_index);

    let sensors = shared.lock().unwrap().available_sensors.clone();
    handle
        .window
        .set_sensor_options(conversions::sensor_options_model(&sensors, false));

    reflect_header(&handle.window, &handle.state);
    reflect_widgets_model(&handle.window, &handle.state, shared);
    handle.window.show().ok();

    let version = Arc::new(AtomicU64::new(0));
    request_preview(&handle.window.as_weak(), &handle.state, &version, shared);
}

fn template_widgets_to_model(widgets: &[Widget], sensors: &[SensorInfo]) -> ModelRc<EditorWidget> {
    let items: Vec<EditorWidget> = widgets
        .iter()
        .map(|w| widget_to_editor(w, sensors))
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn sensor_index_for_source(source: &SensorSourceConfig, sensors: &[SensorInfo]) -> i32 {
    match source {
        SensorSourceConfig::Constant { .. } => 0,
        SensorSourceConfig::Command { .. } => sensors.len() as i32,
        _ => {
            let target = source.to_sensor_source();
            sensors
                .iter()
                .position(|s| s.source == target)
                .map(|i| i as i32)
                .unwrap_or(0)
        }
    }
}

fn command_text_for_source(source: &SensorSourceConfig) -> SharedString {
    match source {
        SensorSourceConfig::Command { cmd } => SharedString::from(cmd.as_str()),
        _ => SharedString::default(),
    }
}

fn widget_to_editor(w: &Widget, sensors: &[SensorInfo]) -> EditorWidget {
    let kind_str = w.kind.kind_id();
    let kind_label = WidgetKind::friendly_name_for(kind_str);
    let mut out = EditorWidget {
        id: SharedString::from(w.id.as_str()),
        kind: SharedString::from(kind_str),
        kind_label: SharedString::from(kind_label),
        x: w.x,
        y: w.y,
        width: w.width,
        height: w.height,
        rotation: w.rotation,
        visible: w.visible,
        update_interval_ms: w.update_interval_ms.unwrap_or(1000) as i32,
        text: SharedString::default(),
        font_name: SharedString::from(DEFAULT_FONT_LABEL),
        font_size: 32.0,
        color_r: 255,
        color_g: 255,
        color_b: 255,
        color_a: 255,
        align: SharedString::from("center"),
        format: SharedString::from("{:.0}"),
        unit: SharedString::default(),
        source_index: 0,
        command: SharedString::default(),
        value_min: 0.0,
        value_max: 100.0,
        start_angle: 0.0,
        sweep_angle: 270.0,
        inner_radius_pct: 0.78,
        bg_r: 40,
        bg_g: 40,
        bg_b: 40,
        bg_a: 255,
        tick_count: 10,
        show_gauge: true,
        show_needle: true,
        needle_width: 14.0,
        needle_length_pct: 95,
        needle_color_r: 255,
        needle_color_g: 255,
        needle_color_b: 255,
        needle_color_a: 255,
        tick_color_r: 120,
        tick_color_g: 140,
        tick_color_b: 160,
        tick_color_a: 255,
        needle_border_r: 174,
        needle_border_g: 10,
        needle_border_b: 16,
        needle_border_a: 255,
        needle_border_width: 1.5,
        show_labels: true,
        image_path: SharedString::default(),
        opacity: 1.0,
        fps: w.fps.unwrap_or(30.0),
    };
    match &w.kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
        } => {
            out.text = SharedString::from(text.as_str());
            out.font_name = SharedString::from(font_ref_to_label(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.color_a = color[3] as i32;
            out.align = SharedString::from(text_align_name(*align));
        }
        WidgetKind::ValueText {
            source,
            format,
            unit,
            font,
            font_size,
            color,
            align,
            value_min,
            value_max,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.format = SharedString::from(format.as_str());
            out.unit = SharedString::from(unit.as_str());
            out.font_name = SharedString::from(font_ref_to_label(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.color_a = color[3] as i32;
            out.align = SharedString::from(text_align_name(*align));
            out.value_min = *value_min;
            out.value_max = *value_max;
        }
        WidgetKind::RadialGauge {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.inner_radius_pct = *inner_radius_pct;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
        }
        WidgetKind::VerticalBar {
            source,
            value_min,
            value_max,
            background_color,
            ..
        }
        | WidgetKind::HorizontalBar {
            source,
            value_min,
            value_max,
            background_color,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
        }
        WidgetKind::Speedometer {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            background_color,
            tick_count,
            show_gauge,
            show_needle,
            needle_width,
            needle_length_pct,
            needle_border_color,
            needle_border_width,
            ..
        } => {
            out.source_index = sensor_index_for_source(source, sensors);
            out.command = command_text_for_source(source);
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.tick_count = *tick_count as i32;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.show_gauge = *show_gauge;
            out.show_needle = *show_needle;
            out.needle_width = *needle_width;
            out.needle_length_pct = (*needle_length_pct * 100.0).round() as i32;
            out.needle_color_r = needle_color[0] as i32;
            out.needle_color_g = needle_color[1] as i32;
            out.needle_color_b = needle_color[2] as i32;
            out.needle_color_a = needle_color[3] as i32;
            out.tick_color_r = tick_color[0] as i32;
            out.tick_color_g = tick_color[1] as i32;
            out.tick_color_b = tick_color[2] as i32;
            out.tick_color_a = tick_color[3] as i32;
            out.needle_border_r = needle_border_color[0] as i32;
            out.needle_border_g = needle_border_color[1] as i32;
            out.needle_border_b = needle_border_color[2] as i32;
            out.needle_border_a = needle_border_color[3] as i32;
            out.needle_border_width = *needle_border_width;
        }
        WidgetKind::CoreBars {
            background_color,
            show_labels,
            ..
        } => {
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
            out.bg_a = background_color[3] as i32;
            out.show_labels = *show_labels;
        }
        WidgetKind::Image { path, opacity, .. } => {
            out.image_path = SharedString::from(path.display().to_string());
            out.opacity = *opacity;
        }
        WidgetKind::Video { path, opacity, .. } => {
            out.image_path = SharedString::from(path.display().to_string());
            out.opacity = *opacity;
        }
    }
    out
}

fn font_ref_to_label(f: &FontRef) -> String {
    font_label_for_path(f.path.as_deref())
}

fn label_to_font_ref(label: &str) -> FontRef {
    FontRef {
        path: font_path_for_label(label),
    }
}

fn text_align_name(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}

fn make_default_widget(id: &str, kind_str: &str, cx: f32, cy: f32) -> Widget {
    let kind = match kind_str {
        "label" => WidgetKind::Label {
            text: "Label".into(),
            font: FontRef::default(),
            font_size: 32.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
        },
        "value_text" => WidgetKind::ValueText {
            source: SensorSourceConfig::CpuUsage,
            format: "{:.0}".into(),
            unit: "%".into(),
            font: FontRef::default(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
            value_min: 0.0,
            value_max: 100.0,
            ranges: default_ranges(),
        },
        "radial_gauge" => WidgetKind::RadialGauge {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 135.0,
            sweep_angle: 270.0,
            inner_radius_pct: 0.78,
            background_color: [40, 40, 40, 255],
            ranges: default_ranges(),
        },
        "vertical_bar" => WidgetKind::VerticalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40, 255],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "horizontal_bar" => WidgetKind::HorizontalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40, 255],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "speedometer" => WidgetKind::Speedometer {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 180.0,
            sweep_angle: 180.0,
            needle_color: [255, 255, 255, 255],
            tick_color: [120, 140, 160, 255],
            tick_count: 10,
            background_color: [40, 40, 40, 255],
            ranges: default_ranges(),
            show_gauge: true,
            show_needle: true,
            needle_width: 14.0,
            needle_length_pct: 0.95,
            needle_border_color: [174, 10, 16, 255],
            needle_border_width: 1.5,
        },
        "core_bars" => WidgetKind::CoreBars {
            orientation: BarOrientation::Horizontal,
            background_color: [30, 30, 30, 255],
            show_labels: true,
            ranges: default_ranges(),
        },
        "image" => WidgetKind::Image {
            path: std::path::PathBuf::new(),
            opacity: 1.0,
            fit: ImageFit::Stretch,
        },
        "video" => WidgetKind::Video {
            path: std::path::PathBuf::new(),
            loop_playback: true,
            opacity: 1.0,
            fit: ImageFit::Stretch,
        },
        _ => WidgetKind::Label {
            text: "Label".into(),
            font: FontRef::default(),
            font_size: 32.0,
            color: [255, 255, 255, 255],
            align: TextAlign::Center,
        },
    };
    Widget {
        id: id.to_string(),
        kind,
        x: cx,
        y: cy,
        width: 120.0,
        height: 80.0,
        rotation: 0.0,
        visible: true,
        update_interval_ms: None,
        fps: None,
    }
}

fn default_ranges() -> Vec<SensorRange> {
    vec![
        SensorRange {
            max: Some(50.0),
            color: [0, 200, 0],
            alpha: 255,
        },
        SensorRange {
            max: Some(75.0),
            color: [220, 140, 0],
            alpha: 255,
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
            alpha: 255,
        },
    ]
}

fn apply_widget_field(widget: &mut Widget, field: &str, val: &str, sensors: &[SensorInfo]) {
    match field {
        "id" => {
            if !val.trim().is_empty() {
                widget.id = val.trim().to_string();
            }
        }
        "x" => {
            if let Ok(v) = val.parse() {
                widget.x = v;
            }
        }
        "y" => {
            if let Ok(v) = val.parse() {
                widget.y = v;
            }
        }
        "width" => {
            if let Ok(v) = val.parse() {
                widget.width = v;
            }
        }
        "height" => {
            if let Ok(v) = val.parse() {
                widget.height = v;
            }
        }
        "rotation" => {
            if let Ok(v) = val.parse() {
                widget.rotation = v;
            }
        }
        "visible" => widget.visible = val == "true",
        "update_interval_ms" => {
            if let Ok(v) = val.parse::<u64>() {
                widget.update_interval_ms = Some(v.clamp(100, 10_000));
            }
        }
        "fps" => {
            if let Ok(v) = val.parse::<f32>() {
                widget.fps = Some(v);
            }
        }
        _ => apply_kind_field(&mut widget.kind, field, val, sensors),
    }
}

fn apply_kind_field(kind: &mut WidgetKind, field: &str, val: &str, sensors: &[SensorInfo]) {
    match kind {
        WidgetKind::Label {
            text,
            font,
            font_size,
            color,
            align,
        } => match field {
            "text" => *text = val.to_string(),
            "font" => *font = label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = parse_u8(val),
            "color_g" => color[1] = parse_u8(val),
            "color_b" => color[2] = parse_u8(val),
            "color_a" => color[3] = parse_u8(val),
            "align" => *align = parse_align(val),
            _ => {}
        },
        WidgetKind::ValueText {
            source,
            format,
            unit,
            font,
            font_size,
            color,
            align,
            value_min,
            value_max,
            ..
        } => match field {
            "text" => {}
            "format" => *format = val.to_string(),
            "unit" => *unit = val.to_string(),
            "font" => *font = label_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = parse_u8(val),
            "color_g" => color[1] = parse_u8(val),
            "color_b" => color[2] = parse_u8(val),
            "color_a" => color[3] = parse_u8(val),
            "align" => *align = parse_align(val),
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            _ => {}
        },
        WidgetKind::RadialGauge {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct: _,
            background_color,
            ranges: _,
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "start_angle" => {
                if let Ok(v) = val.parse() {
                    *start_angle = v;
                }
            }
            "sweep_angle" => {
                if let Ok(v) = val.parse() {
                    *sweep_angle = v;
                }
            }
            "bg_r" => background_color[0] = parse_u8(val),
            "bg_g" => background_color[1] = parse_u8(val),
            "bg_b" => background_color[2] = parse_u8(val),
            "bg_a" => background_color[3] = parse_u8(val),
            _ => {}
        },
        WidgetKind::VerticalBar {
            source,
            value_min,
            value_max,
            background_color,
            ..
        }
        | WidgetKind::HorizontalBar {
            source,
            value_min,
            value_max,
            background_color,
            ..
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "bg_r" => background_color[0] = parse_u8(val),
            "bg_g" => background_color[1] = parse_u8(val),
            "bg_b" => background_color[2] = parse_u8(val),
            "bg_a" => background_color[3] = parse_u8(val),
            _ => {}
        },
        WidgetKind::Speedometer {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            background_color,
            show_gauge,
            show_needle,
            needle_width,
            needle_length_pct,
            needle_border_color,
            needle_border_width,
            ..
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
                }
            }
            "command" => {
                if let SensorSourceConfig::Command { cmd } = source {
                    *cmd = val.to_string();
                } else {
                    *source = SensorSourceConfig::Command {
                        cmd: val.to_string(),
                    };
                }
            }
            "value_min" => {
                if let Ok(v) = val.parse() {
                    *value_min = v;
                }
            }
            "value_max" => {
                if let Ok(v) = val.parse() {
                    *value_max = v;
                }
            }
            "start_angle" => {
                if let Ok(v) = val.parse() {
                    *start_angle = v;
                }
            }
            "sweep_angle" => {
                if let Ok(v) = val.parse() {
                    *sweep_angle = v;
                }
            }
            "show_gauge" => *show_gauge = val == "true",
            "show_needle" => *show_needle = val == "true",
            "needle_width" => {
                if let Ok(v) = val.parse() {
                    *needle_width = v;
                }
            }
            "needle_length_pct" => {
                if let Ok(v) = val.parse::<f32>() {
                    *needle_length_pct = (v / 100.0).clamp(0.1, 1.5);
                }
            }
            "needle_color_r" => needle_color[0] = parse_u8(val),
            "needle_color_g" => needle_color[1] = parse_u8(val),
            "needle_color_b" => needle_color[2] = parse_u8(val),
            "needle_color_a" => needle_color[3] = parse_u8(val),
            "tick_color_r" => tick_color[0] = parse_u8(val),
            "tick_color_g" => tick_color[1] = parse_u8(val),
            "tick_color_b" => tick_color[2] = parse_u8(val),
            "tick_color_a" => tick_color[3] = parse_u8(val),
            "needle_border_r" => needle_border_color[0] = parse_u8(val),
            "needle_border_g" => needle_border_color[1] = parse_u8(val),
            "needle_border_b" => needle_border_color[2] = parse_u8(val),
            "needle_border_a" => needle_border_color[3] = parse_u8(val),
            "needle_border_width" => {
                if let Ok(v) = val.parse() {
                    *needle_border_width = v;
                }
            }
            "bg_r" => background_color[0] = parse_u8(val),
            "bg_g" => background_color[1] = parse_u8(val),
            "bg_b" => background_color[2] = parse_u8(val),
            "bg_a" => background_color[3] = parse_u8(val),
            _ => {}
        },
        WidgetKind::CoreBars {
            background_color,
            show_labels,
            ..
        } => match field {
            "show_labels" => *show_labels = val == "true",
            "bg_r" => background_color[0] = parse_u8(val),
            "bg_g" => background_color[1] = parse_u8(val),
            "bg_b" => background_color[2] = parse_u8(val),
            "bg_a" => background_color[3] = parse_u8(val),
            _ => {}
        },
        WidgetKind::Image { path, .. } | WidgetKind::Video { path, .. } => match field {
            "path" => *path = std::path::PathBuf::from(val),
            _ => {}
        },
    }
}

fn parse_u8(s: &str) -> u8 {
    s.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8
}

fn widget_ranges_mut(kind: &mut WidgetKind) -> Option<&mut Vec<SensorRange>> {
    match kind {
        WidgetKind::RadialGauge { ranges, .. }
        | WidgetKind::VerticalBar { ranges, .. }
        | WidgetKind::HorizontalBar { ranges, .. }
        | WidgetKind::Speedometer { ranges, .. }
        | WidgetKind::CoreBars { ranges, .. }
        | WidgetKind::ValueText { ranges, .. } => Some(ranges),
        _ => None,
    }
}

fn widget_ranges(kind: &WidgetKind) -> Option<&[SensorRange]> {
    match kind {
        WidgetKind::RadialGauge { ranges, .. }
        | WidgetKind::VerticalBar { ranges, .. }
        | WidgetKind::HorizontalBar { ranges, .. }
        | WidgetKind::Speedometer { ranges, .. }
        | WidgetKind::CoreBars { ranges, .. }
        | WidgetKind::ValueText { ranges, .. } => Some(ranges.as_slice()),
        _ => None,
    }
}

fn apply_range_field(range: &mut SensorRange, field: &str, val: &str) {
    match field {
        "max" => {
            if let Ok(v) = val.parse::<i32>() {
                range.max = if v < 0 {
                    None
                } else {
                    Some((v.clamp(0, 100)) as f32)
                };
            }
        }
        "color_r" => range.color[0] = parse_u8(val),
        "color_g" => range.color[1] = parse_u8(val),
        "color_b" => range.color[2] = parse_u8(val),
        "color_a" => range.alpha = parse_u8(val),
        _ => {}
    }
}

fn ranges_to_editor(ranges: &[SensorRange]) -> ModelRc<EditorRange> {
    let items: Vec<EditorRange> = ranges
        .iter()
        .map(|r| EditorRange {
            max_pct: r.max.map(|v| v as i32).unwrap_or(-1),
            color_r: r.color[0] as i32,
            color_g: r.color[1] as i32,
            color_b: r.color[2] as i32,
            color_a: r.alpha as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn reflect_ranges(editor: &TemplateEditorWindow, state: &SharedEditor) {
    let ranges = {
        let st = state.lock();
        let idx = st.selected_widget;
        if idx < 0 {
            None
        } else {
            st.template
                .as_ref()
                .and_then(|t| t.widgets.get(idx as usize))
                .and_then(|w| widget_ranges(&w.kind).map(|r| r.to_vec()))
        }
    };
    let model = match ranges {
        Some(r) => ranges_to_editor(&r),
        None => ModelRc::new(VecModel::<EditorRange>::default()),
    };
    editor.set_selected_ranges(model);
}

fn parse_align(s: &str) -> TextAlign {
    match s {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

fn parse_sensor_source(label: &str, sensors: &[SensorInfo]) -> Option<SensorSourceConfig> {
    if label == "Custom command" {
        return Some(SensorSourceConfig::Command { cmd: String::new() });
    }
    let idx: usize = label.split('.').next()?.parse().ok()?;
    if idx == 0 {
        return None;
    }
    let sensor = sensors.get(idx - 1)?;
    Some(match &sensor.source {
        SensorSource::Hwmon {
            name,
            label,
            device_path,
        } => SensorSourceConfig::Hwmon {
            name: name.clone(),
            label: label.clone(),
            device_path: device_path.clone(),
        },
        SensorSource::NvidiaGpu { gpu_index } => SensorSourceConfig::NvidiaGpu {
            gpu_index: *gpu_index,
        },
        SensorSource::WirelessCoolant { device_id } => SensorSourceConfig::WirelessCoolant {
            device_id: device_id.clone(),
        },
        SensorSource::Command { cmd } => SensorSourceConfig::Command { cmd: cmd.clone() },
        SensorSource::CpuUsage => SensorSourceConfig::CpuUsage,
        SensorSource::MemUsage => SensorSourceConfig::MemUsage,
        SensorSource::MemUsed => SensorSourceConfig::MemUsed,
        SensorSource::MemFree => SensorSourceConfig::MemFree,
    })
}

// Swap base dims and rotate widgets in-place. `to_rotated` direction:
// true → 90° CW (e.g. 480×1920 → 1920×480), false → 90° CCW (inverse).
fn apply_rotation_swap(tpl: &mut LcdTemplate, to_rotated: bool) {
    let old_w = tpl.base_width as f32;
    let old_h = tpl.base_height as f32;
    tpl.base_width = old_h as u32;
    tpl.base_height = old_w as u32;
    for w in tpl.widgets.iter_mut() {
        let (cx, cy, ww, wh) = (w.x, w.y, w.width, w.height);
        if to_rotated {
            w.x = old_h - cy;
            w.y = cx;
        } else {
            w.x = cy;
            w.y = old_w - cx;
        }
        w.width = wh;
        w.height = ww;
    }
}

fn editor_color_from_state(st: &EditorState) -> [u8; 4] {
    if let Some(tpl) = &st.template {
        if let TemplateBackground::Color { rgb } = tpl.background {
            return rgb;
        }
    }
    [0, 0, 0, 255]
}

fn reflect_header(editor: &TemplateEditorWindow, state: &SharedEditor) {
    let st = state.lock();
    let Some(tpl) = st.template.as_ref() else {
        return;
    };
    editor.set_template_id(SharedString::from(tpl.id.as_str()));
    editor.set_template_name(SharedString::from(tpl.name.as_str()));
    editor.set_base_width(tpl.base_width as i32);
    editor.set_base_height(tpl.base_height as i32);
    editor.set_rotated(tpl.rotated);
    match &tpl.background {
        TemplateBackground::Color { rgb } => {
            editor.set_bg_type(SharedString::from("color"));
            editor.set_bg_r(rgb[0] as i32);
            editor.set_bg_g(rgb[1] as i32);
            editor.set_bg_b(rgb[2] as i32);
            editor.set_bg_image_path(SharedString::default());
        }
        TemplateBackground::Image { path } => {
            editor.set_bg_type(SharedString::from("image"));
            editor.set_bg_image_path(SharedString::from(path.display().to_string()));
        }
        TemplateBackground::Builtin { asset } => {
            editor.set_bg_type(SharedString::from("builtin"));
            editor.set_bg_image_path(SharedString::from(format!("builtin:{:?}", asset)));
        }
    }
    editor.set_current_preset_label(SharedString::from(
        screen_preset_label(tpl.base_width, tpl.base_height).as_str(),
    ));
}

fn reflect_widget_row(
    editor: &TemplateEditorWindow,
    state: &SharedEditor,
    shared: &Shared,
    idx: usize,
) {
    let widget_clone = {
        let st = state.lock();
        let Some(tpl) = st.template.as_ref() else {
            return;
        };
        match tpl.widgets.get(idx) {
            Some(w) => w.clone(),
            None => return,
        }
    };
    let sensors = shared.lock().unwrap().available_sensors.clone();
    let editor_widget = widget_to_editor(&widget_clone, &sensors);
    let model = editor.get_widgets();
    if idx < model.row_count() {
        model.set_row_data(idx, editor_widget.clone());
    }
    if editor.get_selected_index() == idx as i32 {
        editor.set_selected_widget(editor_widget);
    }
}

fn select_widget(editor: &TemplateEditorWindow, state: &SharedEditor, shared: &Shared, idx: i32) {
    let widget = {
        let st = state.lock();
        st.template
            .as_ref()
            .and_then(|t| t.widgets.get(idx as usize).cloned())
    };
    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_selected_index(idx);
    if let Some(w) = widget {
        editor.set_selected_widget(widget_to_editor(&w, &sensors));
    } else {
        editor.set_selected_widget(blank_editor_widget());
    }
    reflect_ranges(editor, state);
}

fn blank_editor_widget() -> EditorWidget {
    EditorWidget {
        id: SharedString::default(),
        kind: SharedString::default(),
        kind_label: SharedString::default(),
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        rotation: 0.0,
        visible: true,
        update_interval_ms: 1000,
        text: SharedString::default(),
        font_name: SharedString::from(DEFAULT_FONT_LABEL),
        font_size: 32.0,
        color_r: 255,
        color_g: 255,
        color_b: 255,
        color_a: 255,
        align: SharedString::from("center"),
        format: SharedString::from("{:.0}"),
        unit: SharedString::default(),
        source_index: 0,
        command: SharedString::default(),
        value_min: 0.0,
        value_max: 100.0,
        start_angle: 0.0,
        sweep_angle: 270.0,
        inner_radius_pct: 0.78,
        bg_r: 40,
        bg_g: 40,
        bg_b: 40,
        bg_a: 255,
        tick_count: 10,
        show_gauge: true,
        show_needle: true,
        needle_width: 14.0,
        needle_length_pct: 95,
        needle_color_r: 255,
        needle_color_g: 255,
        needle_color_b: 255,
        needle_color_a: 255,
        tick_color_r: 120,
        tick_color_g: 140,
        tick_color_b: 160,
        tick_color_a: 255,
        needle_border_r: 174,
        needle_border_g: 10,
        needle_border_b: 16,
        needle_border_a: 255,
        needle_border_width: 1.5,
        show_labels: true,
        image_path: SharedString::default(),
        opacity: 1.0,
        fps: 30.0,
    }
}

fn reflect_widgets_model(editor: &TemplateEditorWindow, state: &SharedEditor, shared: &Shared) {
    let (widgets, selected) = {
        let st = state.lock();
        let Some(tpl) = st.template.as_ref() else {
            return;
        };
        (tpl.widgets.clone(), st.selected_widget)
    };
    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_widgets(template_widgets_to_model(&widgets, &sensors));
    editor.set_selected_index(selected);
    if selected >= 0 {
        if let Some(w) = widgets.get(selected as usize) {
            editor.set_selected_widget(widget_to_editor(w, &sensors));
        } else {
            editor.set_selected_widget(blank_editor_widget());
        }
    } else {
        editor.set_selected_widget(blank_editor_widget());
    }
    reflect_ranges(editor, state);
}

/// Spawns a preview render in the background. The version counter discards
/// late responses if the user edits faster than the daemon can render.
fn request_preview(
    weak: &slint::Weak<TemplateEditorWindow>,
    state: &SharedEditor,
    version: &Arc<AtomicU64>,
    _shared: &Shared,
) {
    let tpl = {
        let st = state.lock();
        match &st.template {
            Some(t) => t.clone(),
            None => return,
        }
    };
    let my_version = version.fetch_add(1, Ordering::SeqCst) + 1;
    let version = version.clone();
    let weak = weak.clone();

    std::thread::spawn(move || {
        let req = IpcRequest::RenderTemplatePreview {
            template: tpl.clone(),
            width: tpl.base_width,
            height: tpl.base_height,
        };
        let resp = match ipc_client::send_request(&req) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("preview IPC failed: {e}");
                return;
            }
        };
        if version.load(Ordering::SeqCst) != my_version {
            return;
        }
        let bytes = match decode_preview(&resp) {
            Some(b) => b,
            None => return,
        };
        // Per-version filename so Slint doesn't serve a cached image.
        let tmp = std::env::temp_dir().join(format!(
            "lianli-preview-{}-{}.jpg",
            std::process::id(),
            my_version
        ));
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            tracing::warn!("preview write failed: {e}");
            return;
        }
        let tmp_clone = tmp.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(e) = weak.upgrade() {
                if let Ok(img) = Image::load_from_path(&tmp_clone) {
                    e.set_preview_image(img);
                }
            }
        })
        .ok();
    });
}

fn decode_preview(resp: &lianli_shared::ipc::IpcResponse) -> Option<Vec<u8>> {
    use base64::Engine;
    match resp {
        lianli_shared::ipc::IpcResponse::Ok { data } => {
            let b64 = data.get("jpeg_base64")?.as_str()?;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
        }
        lianli_shared::ipc::IpcResponse::Error { message } => {
            tracing::warn!("preview error: {message}");
            None
        }
    }
}

fn commit_save(state: &SharedEditor, shared: &Shared) -> Result<(), String> {
    let (tpl, target_idx) = {
        let st = state.lock();
        match &st.template {
            Some(t) => (t.clone(), st.target_lcd_index),
            None => return Err("no template open".to_string()),
        }
    };
    if tpl.name.trim().is_empty() {
        return Err("Template name must not be empty".to_string());
    }
    let user_list = {
        let mut gui = shared.lock().unwrap();
        if gui
            .lcd_templates
            .iter()
            .any(|t| t.id != tpl.id && t.name == tpl.name)
        {
            return Err(format!("A template named '{}' already exists.", tpl.name));
        }
        let mut replaced = false;
        for existing in gui.lcd_templates.iter_mut() {
            if existing.id == tpl.id {
                *existing = tpl.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            gui.lcd_templates.push(tpl.clone());
        }
        if let (Some(idx), Some(cfg)) = (target_idx, gui.config.as_mut()) {
            if let Some(lcd) = cfg.lcds.get_mut(idx) {
                lcd.template_id = Some(tpl.id.clone());
            }
        }
        crate::user_templates_only(&gui.lcd_templates)
    };
    crate::send_set_templates(user_list);
    Ok(())
}

fn set_editing(state: &SharedEditor, tpl: LcdTemplate, lcd_index: usize) {
    let mut st = state.lock();
    st.template = Some(tpl);
    st.target_lcd_index = Some(lcd_index);
    st.selected_widget = -1;
    st.preview_version = 0;
}
