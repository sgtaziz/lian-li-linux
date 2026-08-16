//! RGB preset handlers: `SaveRgbPreset`, `DeleteRgbPreset`, `ListRgbPresets`,
//! `ApplyRgbPreset`.

use std::sync::mpsc::Sender;

use lianli_shared::config::AppConfig;
use lianli_shared::ipc::IpcResponse;
use lianli_shared::rgb::{RgbDeviceConfig, RgbMode, RgbPreset, RgbPresetZone, RgbZoneConfig};
use tracing::info;

use crate::ipc::{DaemonState, SharedState};
use crate::service::DaemonEvent;

pub fn save(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    name: String,
    device_id: String,
) -> IpcResponse {
    let zones = {
        let state = state.lock();

        let led_colors = state
            .rgb_controller
            .as_ref()
            .and_then(|rgb| rgb.lock().get_all_zone_colors(&device_id));

        let zone_configs: Vec<_> = state
            .config
            .as_ref()
            .and_then(|c| c.rgb.as_ref())
            .and_then(|r| r.devices.iter().find(|d| d.device_id == device_id))
            .map(|d| d.zones.clone())
            .unwrap_or_default();

        if let Some(led_zones) = led_colors {
            // A zone with captured live LED colors is, by definition, in
            // direct/per-LED mode right now — tag it Direct rather than
            // carrying over whatever effect was configured before, or a
            // later reconciliation will re-engage that stale effect (e.g.
            // Static) ahead of restoring these colors.
            let zones: Vec<RgbPresetZone> = led_zones
                .into_iter()
                .map(|mut z| {
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
        }
    };
    let zones = match zones {
        Some(z) => z,
        None => return IpcResponse::error(format!("device {device_id} not found")),
    };
    let preset = RgbPreset {
        name: name.clone(),
        device_id,
        zones,
    };
    let mut state = state.lock();
    if let Some(existing) = state
        .rgb_presets
        .iter_mut()
        .find(|p| p.name == name && p.device_id == preset.device_id)
    {
        *existing = preset;
    } else {
        state.rgb_presets.push(preset);
    }
    save_and_notify(&mut state, &tx, &name)
}

pub fn delete(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    name: String,
    device_id: String,
) -> IpcResponse {
    let mut state = state.lock();
    let before = state.rgb_presets.len();
    state
        .rgb_presets
        .retain(|p| !(p.name == name && p.device_id == device_id));
    if state.rgb_presets.len() == before {
        return IpcResponse::error(format!("preset '{name}' not found for {device_id}"));
    }
    save_and_notify(&mut state, &tx, &name)
}

pub fn list(state: &SharedState) -> IpcResponse {
    let state = state.lock();
    IpcResponse::ok(&state.rgb_presets)
}

pub fn apply(
    state: &SharedState,
    tx: Sender<DaemonEvent>,
    name: String,
    device_id: String,
) -> IpcResponse {
    let preset = {
        let state = state.lock();
        state
            .rgb_presets
            .iter()
            .find(|p| p.name == name && p.device_id == device_id)
            .cloned()
    };
    let Some(preset) = preset else {
        return IpcResponse::error(format!("preset '{name}' not found for {device_id}"));
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
    // 1. Merge zone effects into the persistent config.
    {
        let app_config = state.config.get_or_insert_with(AppConfig::default);
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
                mb_rgb_sync: false,
                active_preset: None,
                zones: Vec::new(),
            });
            rgb_cfg.devices.last_mut().unwrap()
        };
        dev_cfg.active_preset = Some(name.to_string());
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
    if let Err(e) = super::write_config(&state.config_path, state.config.as_ref().unwrap()) {
        return Err(IpcResponse::error(format!("failed to write config: {e}")));
    }

    // 2. Push per-LED colors to the live RGB controller.
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
                        return Err(IpcResponse::error(format!(
                            "failed to apply preset zone {}: {e}",
                            zone_entry.zone
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Persist the preset list and send an `IpcUpdate` event.
fn save_and_notify(state: &mut DaemonState, tx: &Sender<DaemonEvent>, name: &str) -> IpcResponse {
    match super::write_rgb_presets(&state.presets_path, &state.rgb_presets) {
        Ok(()) => {
            let _ = tx.send(DaemonEvent::IpcUpdate);
            info!("RGB preset '{name}' saved");
            IpcResponse::ok(serde_json::json!(null))
        }
        Err(e) => IpcResponse::error(format!("failed to write presets: {e}")),
    }
}
