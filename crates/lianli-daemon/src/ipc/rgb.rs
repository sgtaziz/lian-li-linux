//! RGB-related IPC handlers: `GetRgbCapabilities`, `SetRgbEffect`,
//! `SetRgbDirect`, `SetRgbFrames`, `SetMbRgbSync`, `SetFanDirection`,
//! `SetLedColor`, `GetZoneColors`.

use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::RgbEffect;
use lianli_shared::ui_message::UiMessage;

use crate::ipc::{rgb_controller_missing, SharedState};

fn failed(key: &str, label: &str, error: impl std::fmt::Display) -> IpcResponse {
    IpcResponse::error_ui(
        format!("{label}: {error}"),
        UiMessage::new(key).param("error", error),
    )
}

fn invalid_config(error: impl std::fmt::Display) -> IpcResponse {
    failed(
        "daemon.rgb.invalidConfig",
        "Invalid RGB configuration",
        error,
    )
}

fn zone_not_found(zone: u8, device_id: &str) -> IpcResponse {
    IpcResponse::error_ui(
        format!("zone {zone} not found on device {device_id}"),
        UiMessage::new("daemon.rgb.zoneNotFound")
            .param("zone", zone)
            .param("device", device_id),
    )
}

pub fn validate_saved_config(
    state: &SharedState,
    config: &lianli_shared::rgb::RgbAppConfig,
) -> Option<IpcResponse> {
    if let Err(error) = lianli_shared::rgb::validate_effect_memory(config) {
        return Some(invalid_config(error));
    }
    if state
        .lock()
        .config
        .as_ref()
        .and_then(|saved| saved.rgb.as_ref())
        == Some(config)
    {
        return None;
    }
    validate_config(state, config)
}

pub fn validate_config(
    state: &SharedState,
    config: &lianli_shared::rgb::RgbAppConfig,
) -> Option<IpcResponse> {
    if let Err(error) = lianli_shared::rgb::validate_effect_memory(config) {
        return Some(invalid_config(error));
    }
    let controller = state.lock().rgb_controller.clone()?;
    let result = controller.lock().validate_config(config);
    result.err().map(invalid_config)
}

pub fn capabilities(state: &SharedState) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
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
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        match rgb.lock().set_effect(&device_id, zone, &effect) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.effectFailed", "RGB effect error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn set_direct(
    state: &SharedState,
    device_id: String,
    zone: u8,
    colors: Vec<[u8; 3]>,
) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        match rgb.lock().set_direct_colors(&device_id, zone, &colors) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.directFailed", "RGB direct error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn set_frames(
    state: &SharedState,
    device_id: String,
    frames: Vec<Vec<[u8; 3]>>,
    interval_ms: u16,
) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        match rgb.lock().set_rgb_frames(&device_id, &frames, interval_ms) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.framesFailed", "RGB frames error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn set_mb_sync(state: &SharedState, device_id: String, enabled: bool) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        match rgb.lock().set_mb_rgb_sync(&device_id, enabled) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.mbSyncFailed", "MB RGB sync error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn set_fan_direction(
    state: &SharedState,
    device_id: String,
    zone: u8,
    swap_lr: bool,
    swap_tb: bool,
) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        match rgb
            .lock()
            .set_fan_direction(&device_id, zone, swap_lr, swap_tb)
        {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.fanDirectionFailed", "Fan direction error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn set_led_color(
    state: &SharedState,
    device_id: String,
    zone: u8,
    led_index: u16,
    color: [u8; 3],
) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        let mut rgb = rgb.lock();
        let mut colors = match rgb.get_zone_colors(&device_id, zone) {
            Some(c) => c,
            None => return zone_not_found(zone, &device_id),
        };
        let idx = led_index as usize;
        if idx >= colors.len() {
            return IpcResponse::error_ui(
                format!(
                    "LED index {led_index} out of range (zone has {} LEDs)",
                    colors.len()
                ),
                UiMessage::new("daemon.rgb.ledIndexOutOfRange")
                    .param("index", led_index)
                    .param("count", colors.len()),
            );
        }
        colors[idx] = color;
        match rgb.set_direct_colors(&device_id, zone, &colors) {
            Ok(()) => IpcResponse::ok(serde_json::json!(null)),
            Err(e) => failed("daemon.rgb.directFailed", "RGB direct error", e),
        }
    } else {
        rgb_controller_missing()
    }
}

pub fn get_zone_colors(state: &SharedState, device_id: String, zone: u8) -> IpcResponse {
    let controller = state.lock().rgb_controller.clone();
    if let Some(rgb) = controller {
        let rgb = rgb.lock();
        match rgb.get_zone_colors(&device_id, zone) {
            Some(colors) => IpcResponse::ok(&colors),
            None => zone_not_found(zone, &device_id),
        }
    } else {
        rgb_controller_missing()
    }
}

/// Closest software engine for wired controllers whose firmware renders the effect.
fn approximate_engine(
    family: lianli_shared::device_id::DeviceFamily,
    name: &str,
) -> Option<(lianli_shared::rgb::RgbRenderFamily, u16)> {
    use lianli_shared::device_id::DeviceFamily;
    use lianli_shared::rgb::RgbRenderFamily;
    match family {
        DeviceFamily::TlFan | DeviceFamily::TlLcd => Some((RgbRenderFamily::Tl, 26)),
        DeviceFamily::Ene6k77 if name.contains("Infinity") => Some((RgbRenderFamily::SlInf, 44)),
        DeviceFamily::Ene6k77 if name.contains("AL") => Some((RgbRenderFamily::Cl, 24)),
        DeviceFamily::Ene6k77 => Some((RgbRenderFamily::Sl, 40)),
        _ => None,
    }
}

pub fn preview(
    state: &SharedState,
    device_id: String,
    config: lianli_shared::rgb::RgbDeviceConfig,
) -> IpcResponse {
    use lianli_media::rgb::preview;
    use lianli_shared::rgb::preview::{RgbPreview, RgbPreviewFidelity};
    use lianli_shared::rgb::{RgbRenderFamily, RgbRenderProfile};

    let Some(controller) = state.lock().rgb_controller.clone() else {
        return rgb_controller_missing();
    };
    let (exact, wired) = {
        let rgb = controller.lock();
        (
            rgb.preview_profile(&device_id),
            rgb.wired_preview_info(&device_id),
        )
    };
    let family = state
        .lock()
        .devices
        .iter()
        .find(|device| {
            device_id == device.device_id
                || device_id
                    .strip_prefix(device.device_id.as_str())
                    .is_some_and(|suffix| suffix.starts_with(':'))
        })
        .map(|device| device.family);
    let approximate = family
        .zip(wired.as_ref())
        .and_then(|(family, (name, zones))| Some((approximate_engine(family, name)?, *zones)))
        .filter(|(_, zones)| (1..=4).contains(zones))
        .map(|((engine, per_zone), zones)| {
            let profile = RgbRenderProfile {
                family: engine,
                fan_count: zones as u8,
                led_count: per_zone * zones as u16,
                right_attach: false,
            };
            (profile, per_zone)
        });
    let (profile, per_zone, mut fidelity) = match (exact, approximate) {
        (Some(profile), _) => {
            let per_zone = (profile.fan_count > 0
                && profile.family != RgbRenderFamily::HydroShiftII)
                .then(|| profile.led_count / u16::from(profile.fan_count));
            (profile, per_zone, RgbPreviewFidelity::Exact)
        }
        (None, Some((profile, per_zone))) => {
            (profile, Some(per_zone), RgbPreviewFidelity::Approximate)
        }
        (None, None) => return IpcResponse::ok(serde_json::json!(null)),
    };
    let rendered = match (fidelity, per_zone) {
        (RgbPreviewFidelity::Exact, per_zone) => match preview::render(profile, &config) {
            Err(_) if per_zone.is_some() => {
                fidelity = RgbPreviewFidelity::Approximate;
                preview::render_zones(profile.family, per_zone.unwrap_or(1), &config.zones)
            }
            rendered => rendered,
        },
        (_, per_zone) => {
            preview::render_zones(profile.family, per_zone.unwrap_or(1), &config.zones)
        }
    };
    let frames = match rendered {
        Ok(Some(frames)) => frames,
        Ok(None) => {
            let Some(zones) = controller.lock().get_all_zone_colors(&device_id) else {
                return IpcResponse::ok(serde_json::json!(null));
            };
            let colors: Vec<_> = zones.into_iter().map(|zone| zone.colors).collect();
            preview::still_frame(&colors, per_zone, usize::from(profile.led_count))
        }
        Err(error) => return failed("daemon.rgb.previewFailed", "RGB preview error", error),
    };
    let Some(layout) = preview::layout(profile) else {
        return IpcResponse::ok(serde_json::json!(null));
    };
    let shown = layout.leds.len();
    let bytes: Vec<u8> = frames
        .frames
        .iter()
        .flat_map(|frame| frame.iter().take(shown).flatten())
        .copied()
        .collect();
    IpcResponse::ok(RgbPreview {
        elements: layout.elements,
        leds: layout.leds,
        frames: super::server::base64_encode(&bytes),
        frame_count: frames.frames.len() as u32,
        interval_ms: frames.interval_ms,
        fidelity,
    })
}

#[cfg(test)]
mod preview_tests {
    use super::approximate_engine;
    use lianli_shared::device_id::DeviceFamily;
    use lianli_shared::rgb::RgbRenderFamily;

    #[test]
    fn firmware_controllers_map_to_their_closest_engine() {
        assert_eq!(
            approximate_engine(DeviceFamily::TlFan, "TL Fan port 1"),
            Some((RgbRenderFamily::Tl, 26))
        );
        assert_eq!(
            approximate_engine(DeviceFamily::Ene6k77, "UNI FAN SL Infinity Group 1"),
            Some((RgbRenderFamily::SlInf, 44))
        );
        assert_eq!(
            approximate_engine(DeviceFamily::Ene6k77, "UNI FAN SL Group 2"),
            Some((RgbRenderFamily::Sl, 40))
        );
        assert_eq!(
            approximate_engine(DeviceFamily::StrimerPlus, "Strimer"),
            None
        );
    }
}
