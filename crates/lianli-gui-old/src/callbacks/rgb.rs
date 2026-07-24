use super::parsing::{parse_rgb_direction, parse_rgb_mode, parse_rgb_scope};
use crate::backend;
use crate::ipc_client;
use crate::{MainWindow, RgbColorData, RgbZoneData, Shared};
use lianli_shared::ipc::IpcRequest;
use lianli_shared::rgb::{
    RgbAppConfig, RgbDeviceConfig, RgbDirection, RgbEffect, RgbMode, RgbScope, RgbZoneConfig,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

pub(crate) fn wire_rgb_callbacks(
    window: &MainWindow,
    backend: &backend::BackendHandle,
    shared: &Shared,
) {
    // ── RGB add/remove color ──
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_add_color(move |dev_id, zone| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                if e.colors.len() < 4 {
                    e.colors.push([255, 255, 255]);
                }
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_colors_in_place(&w, &dev_id, zone, |colors| {
                    if colors.len() < 4 {
                        colors.push(RgbColorData {
                            r: 255,
                            g: 255,
                            b: 255,
                        });
                    }
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_remove_color(move |dev_id, zone, cidx| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let cidx_usize = cidx as usize;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                if e.colors.len() > 1 && cidx_usize < e.colors.len() {
                    e.colors.remove(cidx_usize);
                }
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_colors_in_place(&w, &dev_id, zone, |colors| {
                    if colors.len() > 1 && cidx_usize < colors.len() {
                        colors.remove(cidx_usize);
                    }
                });
            }
        });
    }

    // RGB set mode
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_mode(move |dev_id, zone, mode| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let mode_enum = parse_rgb_mode(&mode);

            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                e.mode = mode_enum;
            });

            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            if let Some(w) = weak.upgrade() {
                let mode = mode.clone();
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.mode = mode.clone();
                    if mode.as_str() == "Direct" && z.led_colors.row_count() == 0 {
                        let base_color = z.colors.row_data(0).unwrap_or(crate::RgbColorData {
                            r: 0,
                            g: 0,
                            b: 0,
                        });
                        let leds: Vec<crate::RgbColorData> = vec![base_color; z.led_count as usize];
                        z.led_colors = slint::ModelRc::new(slint::VecModel::from(leds));
                    }
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_speed(move |dev_id, zone, speed| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                e.speed = speed as u8;
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            // In-place update to avoid destroying expanded-zone state
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.speed = speed;
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_brightness(move |dev_id, zone, brightness| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                e.brightness = brightness as u8;
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            // In-place update to avoid destroying expanded-zone state
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.brightness = brightness;
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_direction(move |dev_id, zone, dir| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                e.direction = parse_rgb_direction(&dir);
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            if let Some(w) = weak.upgrade() {
                let dir = dir.clone();
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.direction = dir.clone();
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_scope(move |dev_id, zone, scope| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                e.scope = parse_rgb_scope(&scope);
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            if let Some(w) = weak.upgrade() {
                let scope = scope.clone();
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.scope = scope.clone();
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_set_color(move |dev_id, zone, cidx, r, g, b| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let effect = with_zone_effect(&shared, &dev_id, zone, |e| {
                let cidx = cidx as usize;
                while e.colors.len() <= cidx {
                    e.colors.push([255, 255, 255]);
                }
                e.colors[cidx] = [r as u8, g as u8, b as u8];
            });
            send_rgb_effect(&tx, &shared, &dev_id, zone, &effect);
            // In-place color update to avoid destroying expanded-zone state
            if let Some(w) = weak.upgrade() {
                let devices = w.get_rgb_devices();
                for di in 0..devices.row_count() {
                    if let Some(dev_data) = devices.row_data(di) {
                        if dev_data.device_id.as_str() == dev_id {
                            // Update target zone
                            if let Some(zone_data) = dev_data.zones.row_data(zone as usize) {
                                zone_data
                                    .colors
                                    .set_row_data(cidx as usize, RgbColorData { r, g, b });
                            }
                            // Broadcast to other zones when synced
                            if zone == 0 && dev_data.synced {
                                for zi in 1..dev_data.zones.row_count() {
                                    if let Some(zd) = dev_data.zones.row_data(zi) {
                                        if (cidx as usize) < zd.colors.row_count() {
                                            zd.colors.set_row_data(
                                                cidx as usize,
                                                RgbColorData { r, g, b },
                                            );
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                w.set_config_dirty(true);
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_toggle_mb_sync(move |dev_id, enabled| {
            let dev_id = dev_id.to_string();
            let base_id = dev_id.split(":port").next().unwrap_or(&dev_id).to_string();
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let rgb = c.rgb.get_or_insert_with(Default::default);
                    // MB sync is controller-wide — update all sibling ports
                    for dev_cfg in &mut rgb.devices {
                        if dev_cfg.device_id.starts_with(&base_id) {
                            dev_cfg.mb_rgb_sync = enabled;
                        }
                    }
                    if !rgb.devices.iter().any(|d| d.device_id == dev_id) {
                        rgb.devices.push(RgbDeviceConfig {
                            device_id: dev_id.clone(),
                            mb_rgb_sync: enabled,
                            active_preset: None,
                            zones: vec![],
                        });
                    }
                }
            }
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                IpcRequest::SetMbRgbSync {
                    device_id: dev_id.clone(),
                    enabled,
                },
            ));
            // In-place update: reflect mb-rgb-sync on all sibling ports
            if let Some(w) = weak.upgrade() {
                let devices = w.get_rgb_devices();
                for di in 0..devices.row_count() {
                    if let Some(mut dev_data) = devices.row_data(di) {
                        if dev_data.device_id.as_str().starts_with(&base_id) {
                            dev_data.mb_rgb_sync = enabled;
                            devices.set_row_data(di, dev_data);
                        }
                    }
                }
                w.set_config_dirty(true);
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_rgb_apply_to_all(move |dev_id| {
            let dev_id = dev_id.to_string();
            let state = shared.lock().unwrap();
            if let Some(ref c) = state.config {
                if let Some(rgb) = &c.rgb {
                    if let Some(dev_cfg) = rgb.devices.iter().find(|d| d.device_id == dev_id) {
                        if let Some(z0) = dev_cfg.zones.first() {
                            let effect = z0.effect.clone();
                            for zone_cfg in &dev_cfg.zones {
                                let _ = tx.send(backend::BackendCommand::IpcRequest(
                                    IpcRequest::SetRgbEffect {
                                        device_id: dev_id.clone(),
                                        zone: zone_cfg.zone_index,
                                        effect: effect.clone(),
                                    },
                                ));
                            }
                        }
                    }
                }
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_toggle_swap_lr(move |dev_id, zone| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let (swap_lr, swap_tb) = {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let rgb = c.rgb.get_or_insert_with(Default::default);
                    let dev = get_or_create_device_config(rgb, &dev_id);
                    let zcfg = get_or_create_zone_config(dev, zone);
                    zcfg.swap_lr = !zcfg.swap_lr;
                    (zcfg.swap_lr, zcfg.swap_tb)
                } else {
                    return;
                }
            };
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                IpcRequest::SetFanDirection {
                    device_id: dev_id.clone(),
                    zone,
                    swap_lr,
                    swap_tb,
                },
            ));
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.swap_lr = swap_lr;
                });
            }
        });
    }

    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_rgb_toggle_swap_tb(move |dev_id, zone| {
            let dev_id = dev_id.to_string();
            let zone = zone as u8;
            let (swap_lr, swap_tb) = {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let rgb = c.rgb.get_or_insert_with(Default::default);
                    let dev = get_or_create_device_config(rgb, &dev_id);
                    let zcfg = get_or_create_zone_config(dev, zone);
                    zcfg.swap_tb = !zcfg.swap_tb;
                    (zcfg.swap_lr, zcfg.swap_tb)
                } else {
                    return;
                }
            };
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                IpcRequest::SetFanDirection {
                    device_id: dev_id.clone(),
                    zone,
                    swap_lr,
                    swap_tb,
                },
            ));
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_in_place(&w, &dev_id, zone, |z| {
                    z.swap_tb = swap_tb;
                });
            }
        });
    }

    // Per-LED color
    {
        let weak = window.as_weak();
        window.on_rgb_set_led_color(move |dev_id, zone, idx, r, g, b| {
            let dev_id_str = dev_id.to_string();
            ipc_client::send_request(&IpcRequest::SetLedColor {
                device_id: dev_id_str,
                zone: zone as u8,
                led_index: idx as u16,
                color: [r as u8, g as u8, b as u8],
            })
            .ok();
            if let Some(w) = weak.upgrade() {
                update_rgb_zone_in_place(&w, dev_id.as_str(), zone as u8, |z| {
                    if let Some(mut c) = z.led_colors.row_data(idx as usize) {
                        c.r = r;
                        c.g = g;
                        c.b = b;
                        z.led_colors.set_row_data(idx as usize, c);
                    }
                });
            }
        });
    }

    // Fill zone
    {
        let weak = window.as_weak();
        window.on_rgb_fill_zone(move |dev_id, zone, r, g, b| {
            let dev_id_str = dev_id.to_string();
            if let Some(w) = weak.upgrade() {
                let led_count = {
                    let devices = w.get_rgb_devices();
                    let mut count = 0usize;
                    for di in 0..devices.row_count() {
                        if let Some(d) = devices.row_data(di) {
                            if d.device_id.as_str() == dev_id.as_str() {
                                if let Some(z) = d.zones.row_data(zone as usize) {
                                    count = z.led_count as usize;
                                }
                                break;
                            }
                        }
                    }
                    count
                };
                if led_count > 0 {
                    let filled: Vec<[u8; 3]> = vec![[r as u8, g as u8, b as u8]; led_count];
                    ipc_client::send_request(&IpcRequest::SetRgbDirect {
                        device_id: dev_id_str,
                        zone: zone as u8,
                        colors: filled,
                    })
                    .ok();
                    update_rgb_zone_in_place(&w, dev_id.as_str(), zone as u8, |z| {
                        let c = crate::RgbColorData { r, g, b };
                        let leds: Vec<crate::RgbColorData> = vec![c; z.led_count as usize];
                        z.led_colors = slint::ModelRc::new(slint::VecModel::from(leds));
                    });
                }
            }
        });
    }

    // Clear zone
    {
        let weak = window.as_weak();
        window.on_rgb_clear_zone(move |dev_id, zone| {
            let dev_id_str = dev_id.to_string();
            if let Some(w) = weak.upgrade() {
                let led_count = {
                    let devices = w.get_rgb_devices();
                    let mut count = 0usize;
                    for di in 0..devices.row_count() {
                        if let Some(d) = devices.row_data(di) {
                            if d.device_id.as_str() == dev_id.as_str() {
                                if let Some(z) = d.zones.row_data(zone as usize) {
                                    count = z.led_count as usize;
                                }
                                break;
                            }
                        }
                    }
                    count
                };
                if led_count > 0 {
                    let cleared: Vec<[u8; 3]> = vec![[0, 0, 0]; led_count];
                    ipc_client::send_request(&IpcRequest::SetRgbDirect {
                        device_id: dev_id_str,
                        zone: zone as u8,
                        colors: cleared,
                    })
                    .ok();
                    update_rgb_zone_in_place(&w, dev_id.as_str(), zone as u8, |z| {
                        let b = crate::RgbColorData { r: 0, g: 0, b: 0 };
                        let leds: Vec<crate::RgbColorData> = vec![b; z.led_count as usize];
                        z.led_colors = slint::ModelRc::new(slint::VecModel::from(leds));
                    });
                }
            }
        });
    }

    // Save preset
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_rgb_save_preset(move |dev_id, name| {
            // Sync local config to daemon before saving preset so effect state is current
            {
                let state = shared.lock().unwrap();
                if let Some(config) = state.config.clone() {
                    ipc_client::send_request(&IpcRequest::SetConfig { config }).ok();
                }
            }
            ipc_client::send_request(&IpcRequest::SaveRgbPreset {
                name: name.to_string(),
                device_id: dev_id.to_string(),
            })
            .ok();
            let _ = tx.send(backend::BackendCommand::ReloadConfig);
        });
    }

    // Apply preset
    {
        let tx = backend.tx.clone();
        window.on_rgb_apply_preset(move |dev_id, name| {
            ipc_client::send_request(&IpcRequest::ApplyRgbPreset {
                name: name.to_string(),
                device_id: dev_id.to_string(),
            })
            .ok();
            // Reload config so the GUI picks up the effect changes written by the daemon
            let _ = tx.send(backend::BackendCommand::ReloadConfig);
        });
    }

    // Delete preset
    {
        let tx = backend.tx.clone();
        window.on_rgb_delete_preset(move |dev_id, name| {
            ipc_client::send_request(&IpcRequest::DeleteRgbPreset {
                name: name.to_string(),
                device_id: dev_id.to_string(),
            })
            .ok();
            let _ = tx.send(backend::BackendCommand::ReloadConfig);
        });
    }
}

/// Get or update an RGB zone's effect in the shared state, returning the updated effect.
pub(super) fn with_zone_effect(
    shared: &Shared,
    dev_id: &str,
    zone: u8,
    mutate: impl FnOnce(&mut RgbEffect),
) -> RgbEffect {
    let mut state = shared.lock().unwrap();
    let c = match state.config.as_mut() {
        Some(c) => c,
        None => {
            let mut e = RgbEffect {
                mode: RgbMode::Static,
                colors: vec![[255, 255, 255]],
                speed: 2,
                brightness: 4,
                direction: RgbDirection::Clockwise,
                scope: RgbScope::All,
                disabled: false,
            };
            mutate(&mut e);
            return e;
        }
    };

    let rgb = c.rgb.get_or_insert_with(Default::default);
    let dev = get_or_create_device_config(rgb, dev_id);
    let zcfg = get_or_create_zone_config(dev, zone);
    mutate(&mut zcfg.effect);
    zcfg.effect.clone()
}

/// Check if a device has group zones (scoped: Top/Bottom or Inner/Outer) and return zone count.
pub(super) fn device_group_zone_count(shared: &Shared, dev_id: &str) -> Option<usize> {
    let state = shared.lock().unwrap();
    let cap = state.rgb_caps.iter().find(|c| c.device_id == dev_id)?;
    let has_group = cap.supported_scopes.iter().any(|scopes| {
        scopes.iter().any(|s| {
            matches!(
                s,
                RgbScope::Top | RgbScope::Bottom | RgbScope::Inner | RgbScope::Outer
            )
        })
    });
    if has_group {
        Some(cap.zones.len())
    } else {
        None
    }
}

/// Send RGB effect IPC, broadcasting to all zones only for animated (synced) modes.
/// Per-fan modes (Static/Off/Direct with scope All) only send for the target zone.
pub(super) fn send_rgb_effect(
    tx: &std::sync::mpsc::Sender<backend::BackendCommand>,
    shared: &Shared,
    dev_id: &str,
    zone: u8,
    effect: &RgbEffect,
) {
    let is_per_fan = matches!(
        effect.mode,
        RgbMode::Off | RgbMode::Static | RgbMode::Direct
    ) && matches!(effect.scope, RgbScope::All);

    let zones_to_update: Vec<u8> = if zone == 0 && !is_per_fan {
        if let Some(zone_count) = device_group_zone_count(shared, dev_id) {
            // Synced/animated mode: broadcast to all zones
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let rgb = c.rgb.get_or_insert_with(Default::default);
                    let dev = get_or_create_device_config(rgb, dev_id);
                    for z in 1..zone_count as u8 {
                        let zcfg = get_or_create_zone_config(dev, z);
                        zcfg.effect = effect.clone();
                    }
                }
            }
            (0..zone_count as u8).collect()
        } else {
            vec![zone]
        }
    } else {
        vec![zone]
    };

    for z in zones_to_update {
        let _ = tx.send(backend::BackendCommand::IpcRequest(
            IpcRequest::SetRgbEffect {
                device_id: dev_id.to_string(),
                zone: z,
                effect: effect.clone(),
            },
        ));
    }
}

pub(super) fn get_or_create_device_config<'a>(
    rgb: &'a mut RgbAppConfig,
    dev_id: &str,
) -> &'a mut RgbDeviceConfig {
    if !rgb.devices.iter().any(|d| d.device_id == dev_id) {
        rgb.devices.push(RgbDeviceConfig {
            device_id: dev_id.to_string(),
            mb_rgb_sync: false,
            active_preset: None,
            zones: vec![],
        });
    }
    rgb.devices
        .iter_mut()
        .find(|d| d.device_id == dev_id)
        .unwrap()
}

pub(super) fn get_or_create_zone_config(dev: &mut RgbDeviceConfig, zone: u8) -> &mut RgbZoneConfig {
    if !dev.zones.iter().any(|z| z.zone_index == zone) {
        dev.zones.push(RgbZoneConfig {
            zone_index: zone,
            effect: RgbEffect {
                mode: RgbMode::Static,
                colors: vec![[255, 255, 255]],
                speed: 2,
                brightness: 4,
                direction: RgbDirection::Clockwise,
                scope: RgbScope::All,
                disabled: false,
            },
            swap_lr: false,
            swap_tb: false,
        });
    }
    dev.zones.iter_mut().find(|z| z.zone_index == zone).unwrap()
}

/// In-place update of RGB zone field(s), preserving expanded-zone state.
/// When zone 0 on a group-zone device, also propagates to other zones.
/// NOTE: We deliberately avoid calling devices.set_row_data() to update the
/// synced flag, because replacing the device in the outer model causes Slint
/// to re-render the RgbDeviceCard and reset its expanded-zone state.
/// The synced flag updates on full model rebuild (initial load / save).
pub(super) fn update_rgb_zone_in_place(
    w: &MainWindow,
    dev_id: &str,
    zone: u8,
    mutate: impl Fn(&mut RgbZoneData),
) {
    let devices = w.get_rgb_devices();
    for di in 0..devices.row_count() {
        if let Some(dev_data) = devices.row_data(di) {
            if dev_data.device_id.as_str() == dev_id {
                // Update the target zone via zones sub-model (preserves device card state)
                if let Some(mut zone_data) = dev_data.zones.row_data(zone as usize) {
                    mutate(&mut zone_data);
                    dev_data.zones.set_row_data(zone as usize, zone_data);
                }
                // On group-zone devices, propagate zone 0 changes to other zones
                // and update is_synced_zone flags.
                if zone == 0 && dev_data.has_group_zones {
                    if let Some(z0) = dev_data.zones.row_data(0) {
                        let is_per_fan = matches!(z0.mode.as_str(), "Off" | "Static" | "Direct")
                            && (z0.scope.as_str().is_empty() || z0.scope.as_str() == "All");
                        let is_synced = !is_per_fan;
                        for zi in 1..dev_data.zones.row_count() {
                            if let Some(mut zd) = dev_data.zones.row_data(zi) {
                                if is_synced {
                                    mutate(&mut zd);
                                }
                                zd.is_synced_zone = is_synced;
                                dev_data.zones.set_row_data(zi, zd);
                            }
                        }
                    }
                }
                break;
            }
        }
    }
    w.set_config_dirty(true);
}

/// In-place update of a zone's color list (add/remove/modify), preserving expanded-zone state.
/// Rebuilds the zone's colors sub-model and updates via set_row_data on the zones model.
pub(super) fn update_rgb_zone_colors_in_place(
    w: &MainWindow,
    dev_id: &str,
    zone: u8,
    mutate: impl FnOnce(&mut Vec<RgbColorData>),
) {
    let devices = w.get_rgb_devices();
    for di in 0..devices.row_count() {
        if let Some(dev_data) = devices.row_data(di) {
            if dev_data.device_id.as_str() == dev_id {
                if let Some(mut zone_data) = dev_data.zones.row_data(zone as usize) {
                    let mut colors: Vec<RgbColorData> = (0..zone_data.colors.row_count())
                        .filter_map(|i| zone_data.colors.row_data(i))
                        .collect();
                    mutate(&mut colors);
                    zone_data.colors = ModelRc::new(VecModel::from(colors));
                    dev_data.zones.set_row_data(zone as usize, zone_data);
                }
                break;
            }
        }
    }
    w.set_config_dirty(true);
}
