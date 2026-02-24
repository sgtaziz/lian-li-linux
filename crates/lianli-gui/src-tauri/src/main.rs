// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod ipc_client;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::connect_daemon,
            commands::get_socket_path,
            commands::list_devices,
            commands::get_telemetry,
            commands::get_config,
            commands::set_config,
            commands::set_lcd_media,
            commands::set_fan_config,
            commands::get_rgb_capabilities,
            commands::set_rgb_effect,
            commands::set_mb_rgb_sync,
            commands::set_rgb_config,
            commands::pick_media_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
