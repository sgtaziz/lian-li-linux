use super::SharedEditor;
use crate::Shared;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::template::LcdTemplate;

// Swap base dims and rotate widgets in-place. `to_rotated` direction:
// true → 90° CW (e.g. 480×1920 → 1920×480), false → 90° CCW (inverse).
pub(super) fn apply_rotation_swap(tpl: &mut LcdTemplate, to_rotated: bool) {
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

pub(super) fn portablize_for_export(tpl: &mut LcdTemplate) {
    for widget in tpl.widgets.iter_mut() {
        let Some(source) = widget.kind.source_config_mut() else {
            continue;
        };
        if widget.sensor_category.is_some() {
            continue;
        }
        if let Some(cat) = lianli_shared::sensors::infer_sensor_category(source) {
            widget.sensor_category = Some(cat);
            *source = SensorSourceConfig::CpuUsage;
        }
    }
}

pub(super) fn copy_to_clipboard(text: String) -> Result<(), String> {
    use arboard::{Clipboard, SetExtLinux};
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                return;
            }
        };
        let _ = tx.send(Ok(()));
        if let Err(e) = clipboard.set().wait().text(text) {
            tracing::warn!("clipboard set(wait) failed: {e}");
        }
    });
    rx.recv_timeout(std::time::Duration::from_millis(500))
        .map_err(|_| "clipboard thread did not respond".to_string())?
}

pub(super) fn commit_save(state: &SharedEditor, shared: &Shared) -> Result<(), String> {
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

pub(super) fn set_editing(state: &SharedEditor, tpl: LcdTemplate, lcd_index: usize) {
    let mut st = state.lock();
    st.template = Some(tpl);
    st.target_lcd_index = Some(lcd_index);
    st.selected_widget = -1;
    st.preview_version = 0;
}
