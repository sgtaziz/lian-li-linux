//! RGB preset handlers: `SaveRgbPreset`, `DeleteRgbPreset`, `ListRgbPresets`,
//! `ApplyRgbPreset`.

use super::EventSender;

use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::{RgbDeviceConfig, RgbMode, RgbPreset, RgbPresetZone, RgbZoneConfig};
use lianli_shared::ui_message::UiMessage;
use tracing::info;

use crate::ipc::{DaemonState, SharedState};
use crate::service::DaemonEvent;

pub fn save(state: &SharedState, tx: EventSender, name: String, device_id: String) -> IpcResponse {
    let (zones, regions) = {
        let state = state.lock();

        let configured_regions = state
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .and_then(|r| r.devices.iter().find(|d| d.device_id == device_id))
            .and_then(|d| d.regions.clone());

        let (led_colors, rendered_regions) = state
            .rgb_controller
            .as_ref()
            .map(|rgb| {
                let rgb = rgb.lock();
                (
                    rgb.get_all_zone_colors(&device_id),
                    rgb.get_effect_regions(&device_id),
                )
            })
            .unwrap_or_default();
        let regions = rendered_regions.or(configured_regions);

        let zone_configs: Vec<_> = state
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .and_then(|r| r.devices.iter().find(|d| d.device_id == device_id))
            .map(|d| d.zones.clone())
            .unwrap_or_default();

        let zones = if regions.is_some() {
            Some(Vec::new())
        } else if let Some(led_zones) = led_colors {
            let zones: Vec<RgbPresetZone> = led_zones
                .into_iter()
                .map(|mut z| {
                    if z.effect.is_some() {
                        return z;
                    }
                    let mut effect = zone_configs
                        .iter()
                        .find(|zc| zc.zone_index == z.zone)
                        .map(|zc| zc.effect.clone())
                        .unwrap_or_default();
                    effect.mode = RgbMode::Direct;
                    z.effect = Some(effect);
                    z
                })
                .collect();
            Some(zones)
        } else if !zone_configs.is_empty() {
            Some(
                zone_configs
                    .iter()
                    .map(|z| RgbPresetZone {
                        zone: z.zone_index,
                        colors: Vec::new(),
                        effect: Some(z.effect.clone()),
                    })
                    .collect(),
            )
        } else {
            None
        };
        (zones, regions)
    };
    let zones = match zones {
        Some(z) => z,
        None => {
            return IpcResponse::error_ui(
                format!("device {device_id} not found"),
                UiMessage::new("daemon.presets.deviceNotFound").param("device", &device_id),
            )
        }
    };
    let preset = RgbPreset {
        name: name.clone(),
        regions,
        device_id,
        zones,
    };
    let mut state = state.lock();
    let mut presets = state.rgb_presets.clone();
    if let Some(existing) = presets
        .iter_mut()
        .find(|p| p.name == name && p.device_id == preset.device_id)
    {
        *existing = preset;
    } else {
        presets.push(preset);
    }
    save_and_notify(&mut state, &tx, &name, presets)
}

pub fn delete(
    state: &SharedState,
    tx: EventSender,
    name: String,
    device_id: String,
) -> IpcResponse {
    let mut state = state.lock();
    let mut presets = state.rgb_presets.clone();
    let before = presets.len();
    presets.retain(|p| !(p.name == name && p.device_id == device_id));
    if presets.len() == before {
        return preset_not_found(&name, &device_id);
    }
    save_and_notify(&mut state, &tx, &name, presets)
}

pub fn list(state: &SharedState) -> IpcResponse {
    let state = state.lock();
    IpcResponse::ok(&state.rgb_presets)
}

pub fn apply(state: &SharedState, tx: EventSender, name: String, device_id: String) -> IpcResponse {
    let preset = {
        let state = state.lock();
        state
            .rgb_presets
            .iter()
            .find(|p| p.name == name && p.device_id == device_id)
            .cloned()
    };
    let Some(preset) = preset else {
        return preset_not_found(&name, &device_id);
    };

    let mut state = state.lock();
    if let Err(resp) = apply_config_and_leds(&mut state, &preset, &name) {
        return resp;
    }
    let _ = tx.send(DaemonEvent::IpcUpdate);
    info!("RGB preset '{name}' applied to {}", preset.device_id);
    IpcResponse::ok(serde_json::json!(null))
}

/// Apply the preset's effect configs to `state.config` and push per-LED colors
/// to the running RGB controller. Persists config to disk.
fn apply_config_and_leds(
    state: &mut DaemonState,
    preset: &RgbPreset,
    name: &str,
) -> Result<(), IpcResponse> {
    let mut app_config = state.config.clone().unwrap_or_default();
    {
        let rgb_cfg = app_config.rgb.get_or_insert_with(Default::default);
        let dev_cfg = if let Some(d) = rgb_cfg
            .devices
            .iter_mut()
            .find(|d| d.device_id == preset.device_id)
        {
            d
        } else {
            rgb_cfg.devices.push(RgbDeviceConfig {
                device_id: preset.device_id.clone(),
                fan_led_count: None,
                mb_rgb_sync: false,
                active_preset: None,
                zones: Vec::new(),
                regions: None,
                effect_memory: Vec::new(),
                port_merge: None,
                reverse_port: false,
            });
            rgb_cfg.devices.last_mut().unwrap()
        };
        dev_cfg.active_preset = Some(name.to_string());
        dev_cfg.regions = preset.regions.clone();
        for zone_entry in &preset.zones {
            if let Some(effect) = &zone_entry.effect {
                if let Some(z) = dev_cfg
                    .zones
                    .iter_mut()
                    .find(|z| z.zone_index == zone_entry.zone)
                {
                    z.effect = effect.clone();
                } else {
                    dev_cfg.zones.push(RgbZoneConfig {
                        zone_index: zone_entry.zone,
                        effect: effect.clone(),
                        swap_lr: false,
                        swap_tb: false,
                    });
                }
            }
        }
    }
    if let Err(e) = super::write_config(&state.config_path, &app_config) {
        return Err(super::config_write_failed(e));
    }
    state.config = Some(app_config);

    let has_led_colors = preset.zones.iter().any(|z| !z.colors.is_empty());
    if has_led_colors {
        if let Some(ref rgb) = state.rgb_controller {
            let mut rgb = rgb.lock();
            for zone_entry in &preset.zones {
                if !zone_entry.colors.is_empty() {
                    if let Err(e) = rgb.set_direct_colors(
                        &preset.device_id,
                        zone_entry.zone,
                        &zone_entry.colors,
                    ) {
                        return Err(IpcResponse::error_ui(
                            format!("failed to apply preset zone {}: {e}", zone_entry.zone),
                            UiMessage::new("daemon.presets.applyZoneFailed")
                                .param("zone", zone_entry.zone)
                                .param("error", e),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Persist the preset list and send an `IpcUpdate` event.
fn save_and_notify(
    state: &mut DaemonState,
    tx: &EventSender,
    name: &str,
    presets: Vec<RgbPreset>,
) -> IpcResponse {
    match super::write_rgb_presets(&state.presets_path, &presets) {
        Ok(()) => {
            state.rgb_presets = presets;
            let _ = tx.send(DaemonEvent::IpcUpdate);
            info!("RGB preset '{name}' saved");
            IpcResponse::ok(serde_json::json!(null))
        }
        Err(e) => IpcResponse::error_ui(
            format!("failed to write presets: {e}"),
            UiMessage::new("daemon.presets.writeFailed").param("error", e),
        ),
    }
}

fn preset_not_found(name: &str, device_id: &str) -> IpcResponse {
    IpcResponse::error_ui(
        format!("preset '{name}' not found for {device_id}"),
        UiMessage::new("daemon.presets.notFound")
            .param("name", name)
            .param("device", device_id),
    )
}
