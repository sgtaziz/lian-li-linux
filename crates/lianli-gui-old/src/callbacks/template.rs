use crate::Shared;

pub(super) fn duplicate_current_template(shared: &Shared, idx: usize) {
    let user_list: Option<Vec<lianli_shared::template::LcdTemplate>> = {
        let mut state = shared.lock().unwrap();
        let current_id = state
            .config
            .as_ref()
            .and_then(|c| c.lcds.get(idx))
            .and_then(|lcd| lcd.template_id.clone());
        let source = current_id
            .as_ref()
            .and_then(|id| state.lcd_templates.iter().find(|t| &t.id == id).cloned());
        if let Some(source) = source {
            let mut copy = source.clone();
            copy.id = crate::generate_template_id("user");
            copy.name = crate::next_unique_name(&source.name, &state.lcd_templates);
            let new_id = copy.id.clone();
            state.lcd_templates.push(copy);
            if let Some(ref mut c) = state.config {
                if let Some(lcd) = c.lcds.get_mut(idx) {
                    lcd.template_id = Some(new_id);
                }
            }
            Some(crate::user_templates_only(&state.lcd_templates))
        } else {
            None
        }
    };
    if let Some(list) = user_list {
        crate::send_set_templates(list);
    }
}

pub(crate) fn strip_copy_suffix(name: &str) -> &str {
    if let Some(idx) = name.rfind(" (Copy") {
        let tail = &name[idx + 6..];
        if tail == ")" || (tail.starts_with(' ') && tail.ends_with(')')) {
            return &name[..idx];
        }
    }
    name
}

pub(super) fn delete_current_template(shared: &Shared, idx: usize) {
    let user_list = {
        let mut state = shared.lock().unwrap();
        let target_id = state
            .config
            .as_ref()
            .and_then(|c| c.lcds.get(idx))
            .and_then(|lcd| lcd.template_id.clone());
        let Some(target_id) = target_id else {
            return;
        };
        state.lcd_templates.retain(|t| t.id != target_id);
        if let Some(ref mut c) = state.config {
            for lcd in c.lcds.iter_mut() {
                if lcd.template_id.as_deref() == Some(target_id.as_str()) {
                    lcd.template_id = None;
                }
            }
        }
        crate::user_templates_only(&state.lcd_templates)
    };
    crate::send_set_templates(user_list);
}
