//! LCD template IPC handlers: `GetLcdTemplates`, `SetLcdTemplates`.

use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;
use lianli_shared::template::LcdTemplate;
use tracing::info;

use crate::ipc::SharedState;
use crate::service::DaemonEvent;
use crate::template_store;

pub fn get(state: &SharedState) -> IpcResponse {
    let state = state.lock();
    let sensors = lianli_shared::sensors::enumerate_sensors();
    let all = template_store::all_templates(&state.user_templates, &sensors);
    IpcResponse::ok(&all)
}

pub fn set(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    templates: Vec<LcdTemplate>,
) -> IpcResponse {
    let mut state = state.lock();
    let path = state.templates_path();
    match template_store::save_user_templates(&path, &templates) {
        Ok(()) => {
            state.user_templates = template_store::load_user_templates(&path);
            let sensors = lianli_shared::sensors::enumerate_sensors();
            template_store::regenerate_template_previews(&state.user_templates, &sensors);
            let _ = tx.send(DaemonEvent::IpcUpdate);
            info!("LCD templates updated via IPC");
            IpcResponse::ok(serde_json::json!(null))
        }
        Err(e) => IpcResponse::error(format!("failed to write templates: {e}")),
    }
}
