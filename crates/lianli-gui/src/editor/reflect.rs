use super::{EditorState, SharedEditor};
use crate::{EditorRange, Shared, TemplateEditorWindow};
use lianli_shared::screen::screen_preset_label;
use lianli_shared::template::TemplateBackground;
use slint::{Model, ModelRc, SharedString, VecModel};

pub(super) fn reflect_ranges(editor: &TemplateEditorWindow, state: &SharedEditor) {
    let ranges = {
        let st = state.lock();
        let idx = st.selected_widget;
        if idx < 0 {
            None
        } else {
            st.template
                .as_ref()
                .and_then(|t| t.widgets.get(idx as usize))
                .and_then(|w| super::apply::widget_ranges(&w.kind).map(|r| r.to_vec()))
        }
    };
    let model = match ranges {
        Some(r) => super::apply::ranges_to_editor(&r),
        None => ModelRc::new(VecModel::<EditorRange>::default()),
    };
    editor.set_selected_ranges(model);
}

pub(super) fn editor_color_from_state(st: &EditorState) -> [u8; 4] {
    if let Some(tpl) = &st.template {
        if let TemplateBackground::Color { rgb } = tpl.background {
            return rgb;
        }
    }
    [0, 0, 0, 255]
}

pub(super) fn reflect_header(editor: &TemplateEditorWindow, state: &SharedEditor) {
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
    }
    editor.set_current_preset_label(SharedString::from(
        screen_preset_label(tpl.base_width, tpl.base_height).as_str(),
    ));
}

pub(super) fn reflect_widget_row(
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
    let editor_widget = super::mapping::widget_to_editor(&widget_clone, &sensors);
    let model = editor.get_widgets();
    if idx < model.row_count() {
        model.set_row_data(idx, editor_widget.clone());
    }
    if editor.get_selected_index() == idx as i32 {
        editor.set_selected_widget(editor_widget);
    }
}

pub(super) fn select_widget(
    editor: &TemplateEditorWindow,
    state: &SharedEditor,
    shared: &Shared,
    idx: i32,
) {
    let widget = {
        let st = state.lock();
        st.template
            .as_ref()
            .and_then(|t| t.widgets.get(idx as usize).cloned())
    };
    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_selected_index(idx);
    if let Some(w) = widget {
        editor.set_selected_widget(super::mapping::widget_to_editor(&w, &sensors));
    } else {
        editor.set_selected_widget(super::mapping::blank_editor_widget());
    }
    reflect_ranges(editor, state);
}

pub(super) fn reflect_widgets_model(
    editor: &TemplateEditorWindow,
    state: &SharedEditor,
    shared: &Shared,
) {
    let (widgets, selected) = {
        let st = state.lock();
        let Some(tpl) = st.template.as_ref() else {
            return;
        };
        (tpl.widgets.clone(), st.selected_widget)
    };
    let sensors = shared.lock().unwrap().available_sensors.clone();
    editor.set_widgets(super::mapping::template_widgets_to_model(
        &widgets, &sensors,
    ));
    editor.set_selected_index(selected);
    if selected >= 0 {
        if let Some(w) = widgets.get(selected as usize) {
            editor.set_selected_widget(super::mapping::widget_to_editor(w, &sensors));
        } else {
            editor.set_selected_widget(super::mapping::blank_editor_widget());
        }
    } else {
        editor.set_selected_widget(super::mapping::blank_editor_widget());
    }
    reflect_ranges(editor, state);
}
