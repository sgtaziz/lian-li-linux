//! LCD IPC handlers: `SwitchDisplayMode`, `RenderTemplatePreview`.

use std::sync::mpsc::Sender;

use lianli_media::CustomAsset;
use lianli_shared::ipc::IpcResponse;
use lianli_shared::screen::ScreenInfo;

use crate::ipc::SharedState;
use crate::service::DaemonEvent;

pub fn switch_display_mode(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    device_id: String,
) -> IpcResponse {
    let (family, pid) = {
        let state = state.lock();
        match state.devices.iter().find(|d| d.device_id == device_id) {
            Some(d) => (Some(d.family), d.pid),
            None => (None, 0),
        }
    };
    match family {
        Some(f) if f.is_desktop_mode() => {
            if pid == 0 {
                return IpcResponse::error("device PID not available");
            }
            let _ = tx.send(DaemonEvent::DisplaySwitchToLcd { device_id, pid });
            IpcResponse::ok(serde_json::json!({
                "switched": "to_lcd",
                "message": "Device is rebooting into LCD mode. It will appear shortly."
            }))
        }
        Some(f) if f.supports_display_mode_switch() => {
            // LCD → Desktop: the service loop owns the WinUSB transport.
            let _ = tx.send(DaemonEvent::DisplaySwitch { device_id });
            IpcResponse::ok(serde_json::json!({
                "switched": "to_desktop",
                "message": "Device is switching to desktop mode. It will reboot shortly."
            }))
        }
        Some(_) => IpcResponse::error("device does not support display mode switching"),
        None => IpcResponse::error(format!("device not found: {device_id}")),
    }
}

pub fn render_template_preview(
    template: lianli_shared::template::LcdTemplate,
    width: u32,
    height: u32,
) -> IpcResponse {
    let preview_screen = ScreenInfo {
        width,
        height,
        max_fps: 30,
        jpeg_quality: 90,
        max_payload: 4 * 1024 * 1024,
        h264: false,
        needs_keepalive: false,
        png: false,
    };
    let all_sensors = lianli_shared::sensors::enumerate_sensors();
    match CustomAsset::new(&template, 0.0, &preview_screen, &all_sensors, false) {
        Ok(asset) => {
            asset.seed_preview_history();
            match asset.render_frame(true) {
                Ok(Some(frame)) => IpcResponse::ok(serde_json::json!({
                    "jpeg_base64": super::server::base64_encode(&frame.data),
                })),
                Ok(None) => {
                    let blank = asset.blank_frame();
                    IpcResponse::ok(serde_json::json!({
                        "jpeg_base64": super::server::base64_encode(&blank.data),
                    }))
                }
                Err(e) => IpcResponse::error(format!("preview render failed: {e}")),
            }
        }
        Err(e) => IpcResponse::error(format!("preview asset creation failed: {e}")),
    }
}
