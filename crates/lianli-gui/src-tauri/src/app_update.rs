use crate::{command_error::CommandError, service_operations};
use lianli_shared::ui_message::UiMessage;

fn failure(error: impl std::fmt::Display) -> CommandError {
    let error = error.to_string();
    CommandError::ui(
        &error,
        UiMessage::new("gui.appUpdate.failed").param("error", &error),
    )
}

#[tauri::command]
pub async fn check_package_update() -> Result<lianli_control::app_update::PackageUpdate, CommandError> {
    tauri::async_runtime::spawn_blocking(lianli_control::app_update::check)
        .await
        .map_err(failure)?
        .map_err(failure)
}

#[tauri::command]
pub async fn start_package_update(app: tauri::AppHandle) -> Result<(), CommandError> {
    let operation = service_operations::Operation::begin(app.clone())?;
    let result = tauri::async_runtime::spawn_blocking(move || operation.run_package_update())
        .await
        .map_err(failure)?;
    if result.is_ok() {
        app.exit(0);
    }
    result
}
