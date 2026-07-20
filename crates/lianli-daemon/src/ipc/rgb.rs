//! RGB-related IPC handlers: `GetRgbCapabilities`, `SetRgbEffect`,
//! `SetRgbDirect`, `SetRgbFrames`, `SetMbRgbSync`, `SetFanDirection`,
//! `SetLedColor`, `GetZoneColors`.

use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::RgbEffect;

use crate::ipc::SharedState;

pub fn capabilities(state: &SharedState) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        let caps = rgb.lock().capabilities();
        IpcResponse::ok(&caps)
    } else {
        IpcResponse::ok(serde_json::json!([]))
    }
}

pub fn set_effect(
    state: &SharedState,
    device_id: String,
    zone: u8,
    effect: RgbEffect,
) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        match rgb.lock().set_effect(&device_id, zone, &effect) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("RGB effect error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn set_direct(
    state: &SharedState,
    device_id: String,
    zone: u8,
    colors: Vec<[u8; 3]>,
) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        match rgb.lock().set_direct_colors(&device_id, zone, &colors) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("RGB direct error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn set_frames(
    state: &SharedState,
    device_id: String,
    frames: Vec<Vec<[u8; 3]>>,
    interval_ms: u16,
) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        match rgb.lock().set_rgb_frames(&device_id, &frames, interval_ms) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("RGB frames error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn set_mb_sync(state: &SharedState, device_id: String, enabled: bool) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        match rgb.lock().set_mb_rgb_sync(&device_id, enabled) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("MB RGB sync error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn set_fan_direction(
    state: &SharedState,
    device_id: String,
    zone: u8,
    swap_lr: bool,
    swap_tb: bool,
) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        match rgb
            .lock()
            .set_fan_direction(&device_id, zone, swap_lr, swap_tb)
        {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("Fan direction error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn set_led_color(
    state: &SharedState,
    device_id: String,
    zone: u8,
    led_index: u16,
    color: [u8; 3],
) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        let mut rgb = rgb.lock();
        let mut colors = match rgb.get_zone_colors(&device_id, zone) {
            Some(c) => c,
            None => {
                return IpcResponse::error(format!("zone {zone} not found on device {device_id}"));
            }
        };
        let idx = led_index as usize;
        if idx >= colors.len() {
            return IpcResponse::error(format!(
                "LED index {led_index} out of range (zone has {} LEDs)",
                colors.len()
            ));
        }
        colors[idx] = color;
        match rgb.set_direct_colors(&device_id, zone, &colors) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => IpcResponse::error(format!("RGB direct error: {e}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}

pub fn get_zone_colors(state: &SharedState, device_id: String, zone: u8) -> IpcResponse {
    let state = state.lock();
    if let Some(ref rgb) = state.rgb_controller {
        let rgb = rgb.lock();
        match rgb.get_zone_colors(&device_id, zone) {
            Some(colors) => IpcResponse::ok(&colors),
            None => IpcResponse::error(format!("zone {zone} not found on device {device_id}")),
        }
    } else {
        IpcResponse::error("RGB controller not initialized")
    }
}
