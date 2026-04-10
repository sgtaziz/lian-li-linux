//! Template Editor wiring — instantiates `TemplateEditorWindow`, binds its
//! callbacks to the in-progress editor state, drives preview rendering via
//! `RenderTemplatePreview` IPC, and commits saves through `SetLcdTemplates`.
//!
//! The editor is a separate top-level Slint Window. It owns its own state
//! (`EditorState`) — the main window seeds properties via setters before
//! `show()` and listens for `save-requested`/`cancel` callbacks.

use crate::conversions;
use crate::ipc_client;
use crate::{EditorWidget, MainWindow, Shared, TemplateEditorWindow};
use lianli_shared::ipc::IpcRequest;
use lianli_shared::media::{SensorRange, SensorSourceConfig};
use lianli_shared::screen::{screen_preset_label, screen_presets};
use lianli_shared::sensors::{SensorInfo, SensorSource};
use lianli_shared::template::{
    BarOrientation, BuiltinFont, FontRef, ImageFit, LcdTemplate, TemplateBackground,
    TemplateOrientation, TextAlign, Widget, WidgetKind,
};
use parking_lot::Mutex as PLMutex;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// In-progress template state. The editor edits a clone of the original
/// template and only pushes it back on save. The parent main window
/// assigns `target_lcd_index` when it opens the editor so we know which
/// LcdConfig to point at the new template on save.
#[derive(Debug, Default)]
pub struct EditorState {
    pub template: Option<LcdTemplate>,
    pub target_lcd_index: Option<usize>,
    pub selected_widget: i32,
    /// Monotonically-incrementing version used to discard stale preview
    /// renders when rapid edits pile up.
    pub preview_version: u64,
}

pub type SharedEditor = Arc<PLMutex<EditorState>>;

/// Publicly visible bundle returned by `install()` so main.rs can both show
/// the window and push new editing state into it.
pub struct EditorHandle {
    pub window: TemplateEditorWindow,
    pub state: SharedEditor,
}

/// Install the editor window alongside the main window. Returns an
/// [`EditorHandle`] that `wire_lcd_callbacks` uses to launch the editor for
/// a given LCD index + template.
pub fn install(main: &MainWindow, shared: Shared) -> EditorHandle {
    let editor = TemplateEditorWindow::new().expect("Failed to create template editor window");
    let editor_state: SharedEditor = Arc::new(PLMutex::new(EditorState::default()));
    let preview_version = Arc::new(AtomicU64::new(0));

    // ── Seed static models ──
    let presets: Vec<SharedString> = screen_presets()
        .iter()
        .map(|p| SharedString::from(p.label))
        .collect();
    editor.set_device_presets(ModelRc::new(VecModel::from(presets)));

    // Populate sensor options from current shared state (same format the LCD
    // page uses so the dropdown is consistent).
    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_sensor_options(conversions::sensor_options_model(&sensors, false));

    // ── save-requested ──
    {
        let editor_state = editor_state.clone();
        let shared = shared.clone();
        let editor_weak = editor.as_weak();
        let main_weak = main.as_weak();
        editor.on_save_requested(move || {
            commit_save(&editor_state, &shared);
            if let Some(e) = editor_weak.upgrade() {
                e.hide().ok();
            }
            crate::refresh_lcd_ui(&main_weak, &shared);
        });
    }

    // ── cancel ──
    {
        let editor_weak = editor.as_weak();
        editor.on_cancel(move || {
            if let Some(e) = editor_weak.upgrade() {
                e.hide().ok();
            }
        });
    }

    // ── header-field ──
    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_header_field(move |field, val| {
            {
                let mut st = editor_state.lock();
                let prev_color = editor_color_from_state(&st);
                let Some(tpl) = st.template.as_mut() else {
                    return;
                };
                match field.as_str() {
                    "name" => tpl.name = val.to_string(),
                    "orientation" => {
                        tpl.orientation = if val.as_str() == "landscape" {
                            TemplateOrientation::Landscape
                        } else {
                            TemplateOrientation::Portrait
                        };
                    }
                    "bg_type" => {
                        if val.as_str() == "color" {
                            tpl.background = TemplateBackground::Color { rgb: prev_color };
                        }
                    }
                    _ => {}
                }
            }
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── preset-selected ──
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
                }
            }
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── widget-moved (drag callback) ──
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
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── widget-added ──
    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_added(move |kind| {
            {
                let mut st = editor_state.lock();
                if let Some(tpl) = st.template.as_mut() {
                    let (cx, cy) = (tpl.base_width as f32 / 2.0, tpl.base_height as f32 / 2.0);
                    let id = format!("w-{}", tpl.widgets.len() + 1);
                    tpl.widgets
                        .push(make_default_widget(&id, kind.as_str(), cx, cy));
                    st.selected_widget = (tpl.widgets.len() - 1) as i32;
                }
            }
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── widget-removed ──
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
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── widget-field (inspector edits) ──
    {
        let editor_state = editor_state.clone();
        let editor_weak = editor.as_weak();
        let preview_version = preview_version.clone();
        let shared = shared.clone();
        editor.on_widget_field(move |idx, field, val| {
            if field.as_str() == "_select" {
                editor_state.lock().selected_widget = idx;
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
            reflect_editor_state(&editor_weak, &editor_state);
            request_preview(&editor_weak, &editor_state, &preview_version, &shared);
        });
    }

    // ── pick-bg-image ──
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
                    reflect_editor_state(&editor_weak2, &editor_state2);
                    request_preview(&editor_weak2, &editor_state2, &preview_version2, &shared2);
                }
            });
        });
    }

    // ── pick-widget-image ──
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
                    reflect_editor_state(&editor_weak2, &editor_state2);
                    request_preview(&editor_weak2, &editor_state2, &preview_version2, &shared2);
                }
            });
        });
    }

    let _ = main; // main window ref kept around only for symmetric API

    EditorHandle {
        window: editor,
        state: editor_state,
    }
}

/// Open the editor window, seeding it with the template the caller wants to
/// edit. If `initial` is None, a blank user template is generated.
pub fn open(
    handle: &EditorHandle,
    shared: &Shared,
    lcd_index: usize,
    initial: Option<LcdTemplate>,
) {
    let tpl = initial.unwrap_or_else(crate::make_blank_template);
    set_editing(&handle.state, tpl.clone(), lcd_index);

    // Rebuild the sensor options model (sensor list may have changed since
    // install time — hot-plug, daemon restart, etc.).
    let sensors = shared.lock().unwrap().available_sensors.clone();
    handle
        .window
        .set_sensor_options(conversions::sensor_options_model(&sensors, false));

    seed_editor_from_template(&handle.window, &tpl);
    handle.window.show().ok();

    // Kick off an initial preview render.
    let version = Arc::new(AtomicU64::new(0));
    request_preview(&handle.window.as_weak(), &handle.state, &version, shared);
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn seed_editor_from_template(editor: &TemplateEditorWindow, tpl: &LcdTemplate) {
    editor.set_template_id(SharedString::from(tpl.id.as_str()));
    editor.set_template_name(SharedString::from(tpl.name.as_str()));
    editor.set_base_width(tpl.base_width as i32);
    editor.set_base_height(tpl.base_height as i32);
    editor.set_orientation(SharedString::from(match tpl.orientation {
        TemplateOrientation::Portrait => "portrait",
        TemplateOrientation::Landscape => "landscape",
    }));
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
    }
    editor.set_current_preset_label(SharedString::from(
        screen_preset_label(tpl.base_width, tpl.base_height).as_str(),
    ));
    editor.set_widgets(template_widgets_to_model(&tpl.widgets));
    editor.set_selected_index(-1);
}

fn template_widgets_to_model(widgets: &[Widget]) -> ModelRc<EditorWidget> {
    let items: Vec<EditorWidget> = widgets.iter().map(widget_to_editor).collect();
    ModelRc::new(VecModel::from(items))
}

fn widget_to_editor(w: &Widget) -> EditorWidget {
    let kind_str = widget_kind_name(&w.kind);
    let mut out = EditorWidget {
        id: SharedString::from(w.id.as_str()),
        kind: SharedString::from(kind_str),
        x: w.x,
        y: w.y,
        width: w.width,
        height: w.height,
        rotation: w.rotation,
        visible: w.visible,
        update_interval_ms: w.update_interval_ms.unwrap_or(1000) as i32,
        text: SharedString::default(),
        font_name: SharedString::from("Victor Mono"),
        font_size: 32.0,
        color_r: 255,
        color_g: 255,
        color_b: 255,
        align: SharedString::from("center"),
        format: SharedString::from("{:.0}"),
        unit: SharedString::default(),
        source_index: 0,
        value_min: 0.0,
        value_max: 100.0,
        start_angle: 0.0,
        sweep_angle: 270.0,
        inner_radius_pct: 0.78,
        bg_r: 40,
        bg_g: 40,
        bg_b: 40,
        tick_count: 10,
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
            out.font_name = SharedString::from(font_ref_to_name(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.align = SharedString::from(text_align_name(*align));
        }
        WidgetKind::ValueText {
            format,
            unit,
            font,
            font_size,
            color,
            align,
            ..
        } => {
            out.format = SharedString::from(format.as_str());
            out.unit = SharedString::from(unit.as_str());
            out.font_name = SharedString::from(font_ref_to_name(font));
            out.font_size = *font_size;
            out.color_r = color[0] as i32;
            out.color_g = color[1] as i32;
            out.color_b = color[2] as i32;
            out.align = SharedString::from(text_align_name(*align));
        }
        WidgetKind::RadialGauge {
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ..
        } => {
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.inner_radius_pct = *inner_radius_pct;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
        }
        WidgetKind::VerticalBar {
            value_min,
            value_max,
            background_color,
            ..
        }
        | WidgetKind::HorizontalBar {
            value_min,
            value_max,
            background_color,
            ..
        } => {
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
        }
        WidgetKind::Speedometer {
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            background_color,
            tick_count,
            ..
        } => {
            out.value_min = *value_min;
            out.value_max = *value_max;
            out.start_angle = *start_angle;
            out.sweep_angle = *sweep_angle;
            out.tick_count = *tick_count as i32;
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
        }
        WidgetKind::CoreBars {
            background_color, ..
        } => {
            out.bg_r = background_color[0] as i32;
            out.bg_g = background_color[1] as i32;
            out.bg_b = background_color[2] as i32;
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

fn widget_kind_name(kind: &WidgetKind) -> &'static str {
    match kind {
        WidgetKind::Label { .. } => "label",
        WidgetKind::ValueText { .. } => "value_text",
        WidgetKind::RadialGauge { .. } => "radial_gauge",
        WidgetKind::VerticalBar { .. } => "vertical_bar",
        WidgetKind::HorizontalBar { .. } => "horizontal_bar",
        WidgetKind::Speedometer { .. } => "speedometer",
        WidgetKind::CoreBars { .. } => "core_bars",
        WidgetKind::Image { .. } => "image",
        WidgetKind::Video { .. } => "video",
    }
}

fn font_ref_to_name(f: &FontRef) -> &'static str {
    match f {
        FontRef::Builtin { font } => match font {
            BuiltinFont::VictorMono => "Victor Mono",
            BuiltinFont::JetBrainsMono => "JetBrains Mono",
            BuiltinFont::Digital7 => "Digital 7",
        },
        FontRef::File { .. } => "Victor Mono",
    }
}

fn name_to_font_ref(name: &str) -> FontRef {
    match name {
        "JetBrains Mono" => FontRef::Builtin {
            font: BuiltinFont::JetBrainsMono,
        },
        "Digital 7" => FontRef::Builtin {
            font: BuiltinFont::Digital7,
        },
        _ => FontRef::Builtin {
            font: BuiltinFont::VictorMono,
        },
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
            color: [255, 255, 255],
            align: TextAlign::Center,
        },
        "value_text" => WidgetKind::ValueText {
            source: SensorSourceConfig::CpuUsage,
            format: "{:.0}".into(),
            unit: "%".into(),
            font: FontRef::default(),
            font_size: 48.0,
            color: [255, 255, 255],
            align: TextAlign::Center,
        },
        "radial_gauge" => WidgetKind::RadialGauge {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 135.0,
            sweep_angle: 270.0,
            inner_radius_pct: 0.78,
            background_color: [40, 40, 40],
            ranges: default_ranges(),
        },
        "vertical_bar" => WidgetKind::VerticalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "horizontal_bar" => WidgetKind::HorizontalBar {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            background_color: [40, 40, 40],
            corner_radius: 4.0,
            ranges: default_ranges(),
        },
        "speedometer" => WidgetKind::Speedometer {
            source: SensorSourceConfig::CpuUsage,
            value_min: 0.0,
            value_max: 100.0,
            start_angle: 180.0,
            sweep_angle: 180.0,
            needle_color: [255, 255, 255],
            tick_color: [120, 140, 160],
            tick_count: 10,
            background_color: [40, 40, 40],
        },
        "core_bars" => WidgetKind::CoreBars {
            orientation: BarOrientation::Horizontal,
            color_cold: [0, 200, 0],
            color_hot: [220, 0, 0],
            background_color: [30, 30, 30],
            show_labels: true,
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
            color: [255, 255, 255],
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
        },
        SensorRange {
            max: Some(80.0),
            color: [220, 140, 0],
        },
        SensorRange {
            max: None,
            color: [220, 0, 0],
        },
    ]
}

fn apply_widget_field(widget: &mut Widget, field: &str, val: &str, sensors: &[SensorInfo]) {
    // Shared geometry
    match field {
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
            "font" => *font = name_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = parse_u8(val),
            "color_g" => color[1] = parse_u8(val),
            "color_b" => color[2] = parse_u8(val),
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
        } => match field {
            "text" => { /* ignored for value_text */ }
            "format" => *format = val.to_string(),
            "unit" => *unit = val.to_string(),
            "font" => *font = name_to_font_ref(val),
            "font_size" => {
                if let Ok(v) = val.parse() {
                    *font_size = v;
                }
            }
            "color_r" => color[0] = parse_u8(val),
            "color_g" => color[1] = parse_u8(val),
            "color_b" => color[2] = parse_u8(val),
            "align" => *align = parse_align(val),
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
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
            background_color: _,
            ranges: _,
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
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
            _ => {}
        },
        WidgetKind::VerticalBar {
            source,
            value_min,
            value_max,
            ..
        }
        | WidgetKind::HorizontalBar {
            source,
            value_min,
            value_max,
            ..
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
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
        WidgetKind::Speedometer {
            source,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            ..
        } => match field {
            "source" => {
                if let Some(new) = parse_sensor_source(val, sensors) {
                    *source = new;
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
            _ => {}
        },
        WidgetKind::Image { path, .. } | WidgetKind::Video { path, .. } => match field {
            "path" => *path = std::path::PathBuf::from(val),
            _ => {}
        },
        _ => {}
    }
}

fn parse_u8(s: &str) -> u8 {
    s.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8
}

fn parse_align(s: &str) -> TextAlign {
    match s {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

/// Parse a sensor-picker label ("1. CPU (hwmon)" / "Custom command") back
/// into a `SensorSourceConfig` via the available-sensors index.
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

fn editor_color_from_state(st: &EditorState) -> [u8; 3] {
    if let Some(tpl) = &st.template {
        if let TemplateBackground::Color { rgb } = tpl.background {
            return rgb;
        }
    }
    [10, 14, 22]
}

/// Re-seed the editor's UI properties from the current `EditorState`.
/// Called after every mutation so widget inspector values, list rows, and
/// the header reflect the live template.
fn reflect_editor_state(weak: &slint::Weak<TemplateEditorWindow>, state: &SharedEditor) {
    let (tpl, selected) = {
        let st = state.lock();
        match &st.template {
            Some(t) => (t.clone(), st.selected_widget),
            None => return,
        }
    };
    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(e) = weak.upgrade() {
            seed_editor_from_template(&e, &tpl);
            e.set_selected_index(selected);
        }
    })
    .ok();
}

/// Send a preview-render IPC request on a background thread. Uses a
/// monotonic counter to discard outdated responses when the user edits
/// faster than the daemon can render.
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
        // Render against the template's own base size — keeps the preview
        // aspect matching the authored canvas.
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
        // Discard stale responses.
        if version.load(Ordering::SeqCst) != my_version {
            return;
        }
        let bytes = match decode_preview(&resp) {
            Some(b) => b,
            None => return,
        };
        // Write to a per-version temp file so Slint reloads rather than
        // reusing the cached image.
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

fn commit_save(state: &SharedEditor, shared: &Shared) {
    let (tpl, target_idx) = {
        let st = state.lock();
        match &st.template {
            Some(t) => (t.clone(), st.target_lcd_index),
            None => return,
        }
    };
    let user_list = {
        let mut gui = shared.lock().unwrap();
        // Replace an existing template with the same id, or append.
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
        // Point the target LCD at the saved template.
        if let (Some(idx), Some(cfg)) = (target_idx, gui.config.as_mut()) {
            if let Some(lcd) = cfg.lcds.get_mut(idx) {
                lcd.template_id = Some(tpl.id.clone());
            }
        }
        crate::user_templates_only(&gui.lcd_templates)
    };
    crate::send_set_templates(user_list);
}

// ---------------------------------------------------------------------------
// Integration helpers for main.rs
// ---------------------------------------------------------------------------

/// Set the editor state's "currently editing" template + target LCD index.
/// Called from `open()` before the window is shown.
fn set_editing(state: &SharedEditor, tpl: LcdTemplate, lcd_index: usize) {
    let mut st = state.lock();
    st.template = Some(tpl);
    st.target_lcd_index = Some(lcd_index);
    st.selected_widget = -1;
    st.preview_version = 0;
}
