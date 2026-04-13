mod backend;
mod conversions;
mod editor;
mod ipc_client;
mod state;
mod template_browser;

use lianli_shared::fan::{FanConfig, FanCurve, FanGroup, FanSpeed};
use lianli_shared::ipc::IpcRequest;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::rgb::{
    RgbAppConfig, RgbDeviceConfig, RgbDirection, RgbEffect, RgbMode, RgbScope, RgbZoneConfig,
};
use lianli_shared::sensors::Unit;
use slint::{Model, ModelRc, VecModel};
use std::sync::{Arc, Mutex};

slint::include_modules!();

/// Shared mutable state: config + cached capabilities + devices.
/// Backend thread updates it on load; callbacks mutate config; save sends it.
pub type Shared = Arc<Mutex<state::SharedState>>;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("lianli_gui2=info".parse().unwrap()),
        )
        .init();

    let window = MainWindow::new().expect("Failed to create main window");
    if let Err(e) = slint::set_xdg_app_id("com.sgtaziz.lianlilinux") {
        tracing::warn!("set_xdg_app_id failed: {e}");
    }
    window.set_app_version(env!("CARGO_PKG_VERSION").into());

    // Shared state — backend will populate on first load
    let shared: Shared = Arc::new(Mutex::new(state::SharedState::default()));
    let backend = backend::start(window.as_weak(), shared.clone());

    // ── Refresh devices ──
    {
        let tx = backend.tx.clone();
        window.on_refresh_devices(move || {
            let _ = tx.send(backend::BackendCommand::RefreshDevices);
        });
    }

    // ── Switch display mode ──
    {
        let tx = backend.tx.clone();
        window.on_switch_display_mode(move |device_id| {
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                lianli_shared::ipc::IpcRequest::SwitchDisplayMode {
                    device_id: device_id.to_string(),
                },
            ));
        });
    }

    // ── Bind wireless device ──
    {
        let tx = backend.tx.clone();
        window.on_bind_wireless_device(move |device_id| {
            let mac = device_id
                .to_string()
                .strip_prefix("wireless-unbound:")
                .unwrap_or(&device_id)
                .to_string();
            let _ = tx.send(backend::BackendCommand::IpcRequest(
                lianli_shared::ipc::IpcRequest::BindWirelessDevice { mac },
            ));
        });
    }

    // ── Save config ──
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_save_config(move || {
            let state = shared.lock().unwrap();
            if let Some(ref c) = state.config {
                let _ = tx.send(backend::BackendCommand::SaveConfig(c.clone()));
            }
        });
    }

    // ── Toggle OpenRGB ──
    {
        let tx = backend.tx.clone();
        let shared = shared.clone();
        window.on_toggle_openrgb(move |enabled| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let rgb = c.rgb.get_or_insert_with(Default::default);
                rgb.openrgb_server = enabled;
                let _ = tx.send(backend::BackendCommand::SaveConfig(c.clone()));
            }
        });
    }

    // ── Set default FPS ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_default_fps(move |fps| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                c.default_fps = fps as f32;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set OpenRGB port ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_openrgb_port(move |port| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let rgb = c.rgb.get_or_insert_with(Default::default);
                rgb.openrgb_port = port as u16;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set HID driver ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_set_hid_driver(move |driver| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                c.hid_driver = match driver.as_str() {
                    "Rusb" => lianli_shared::config::HidDriver::Rusb,
                    _ => lianli_shared::config::HidDriver::Hidapi,
                };
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    // ── Set fan update interval ──
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_update_interval(move |ms| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                let fc = c.fans.get_or_insert_with(|| FanConfig {
                    speeds: vec![],
                    update_interval_ms: 500,
                });
                fc.update_interval_ms = ms as u64;
            }
            drop(state);
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

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

    // ── RGB callbacks ──
    wire_rgb_callbacks(&window, &backend, &shared);

    // ── Fan callbacks ──
    wire_fan_callbacks(&window, &backend, &shared);

    let editor_handle = editor::install(&window, shared.clone());
    let browser_handle = template_browser::install(&window, shared.clone());

    wire_lcd_callbacks(&window, &shared, &editor_handle, &browser_handle);

    window.run().expect("Failed to run Slint event loop");
    backend.send(backend::BackendCommand::Shutdown);
}

fn wire_rgb_callbacks(window: &MainWindow, backend: &backend::BackendHandle, shared: &Shared) {
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

fn wire_fan_callbacks(window: &MainWindow, _backend: &backend::BackendHandle, shared: &Shared) {
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_add_curve(move || {
            {
                let mut state = shared.lock().unwrap();
                let default_source = state.available_sensors.first().map(|s| s.source.clone());
                if let Some(ref mut c) = state.config {
                    let n = c.fan_curves.len() + 1;
                    c.fan_curves.push(FanCurve {
                        name: format!("curve-{n}"),
                        temp_source: default_source,
                        temp_command: String::new(),
                        curve: vec![(30.0, 30.0), (50.0, 50.0), (70.0, 80.0), (85.0, 100.0)],
                    });
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_remove_curve(move |idx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if idx < c.fan_curves.len() {
                        c.fan_curves.remove(idx);
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_rename_curve(move |idx, name| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                        curve.name = name.to_string();
                    }
                }
            }
            // Don't rebuild model — would destroy the focused LineEdit.
            // The typed text is already visible. Mark dirty only.
            if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_temp_source(move |idx, display_name| {
            let display = display_name.to_string();
            {
                let mut state = shared.lock().unwrap();
                let source = if display.ends_with("Custom command") {
                    None
                } else {
                    let sensor_idx: usize = display
                        .split('.')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    sensor_idx.checked_sub(1).and_then(|i| {
                        state
                            .available_sensors
                            .iter()
                            .filter(|s| s.unit == Unit::C)
                            .nth(i)
                            .map(|s| s.source.clone())
                    })
                };
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                        curve.temp_source = source;
                        if curve.temp_source.is_some() {
                            curve.temp_command.clear();
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        window.on_fan_set_temp_command(move |idx, cmd| {
            let mut state = shared.lock().unwrap();
            if let Some(ref mut c) = state.config {
                if let Some(curve) = c.fan_curves.get_mut(idx as usize) {
                    curve.temp_command = cmd.to_string();
                }
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_moved(move |cidx, pidx, temp, speed| {
            let temp = temp.round().clamp(20.0, 100.0) as f32;
            let speed = speed.round().clamp(0.0, 100.0) as f32;
            let cidx_u = cidx as usize;
            let pidx_u = pidx as usize;

            // Update shared state, get sorted points for path rebuild
            let sorted = {
                let mut state = shared.lock().unwrap();
                let c = match state.config.as_mut() {
                    Some(c) => c,
                    None => return,
                };
                let curve = match c.fan_curves.get_mut(cidx_u) {
                    Some(curve) => curve,
                    None => return,
                };
                if let Some(pt) = curve.curve.get_mut(pidx_u) {
                    pt.0 = temp;
                    pt.1 = speed;
                }
                let mut sorted = curve.curve.clone();
                sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                sorted
            };

            // Synchronous in-place model update (we're on the UI thread).
            // This preserves the TouchArea so the drag continues.
            if let Some(w) = weak.upgrade() {
                let model = w.get_fan_curves();
                if let Some(mut curve_data) = model.row_data(cidx_u) {
                    // Update inner points model in-place
                    curve_data
                        .points
                        .set_row_data(pidx_u, CurvePoint { temp, speed });
                    // Update segment models
                    curve_data.curve_segments = slint::ModelRc::new(slint::VecModel::from(
                        conversions::build_curve_segments(&sorted),
                    ));
                    curve_data.clamp_segments = slint::ModelRc::new(slint::VecModel::from(
                        conversions::build_clamp_segments(&sorted),
                    ));
                    model.set_row_data(cidx_u, curve_data);
                    w.set_config_dirty(true);
                }
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_added(move |cidx, temp, speed| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(cidx as usize) {
                        curve.curve.push((
                            temp.round().clamp(20.0, 100.0),
                            speed.round().clamp(0.0, 100.0),
                        ));
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_point_removed(move |cidx, pidx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    if let Some(curve) = c.fan_curves.get_mut(cidx as usize) {
                        let pidx = pidx as usize;
                        if pidx < curve.curve.len() {
                            curve.curve.remove(pidx);
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    // Fan speed assignment
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_slot_speed(move |dev_id, slot, val| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            let val = val.to_string();
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(|| FanConfig {
                        speeds: vec![],
                        update_interval_ms: 500,
                    });
                    let group = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id));
                    let group = if let Some(g) = group {
                        g
                    } else {
                        fc.speeds.push(FanGroup {
                            device_id: Some(dev_id.clone()),
                            speeds: [
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                                FanSpeed::Constant(0),
                            ],
                        });
                        fc.speeds.last_mut().unwrap()
                    };

                    let speed: FanSpeed = match val.as_str() {
                        "Off" => FanSpeed::Constant(0),
                        "Constant PWM" => FanSpeed::Constant(128),
                        "MB Sync" => FanSpeed::Curve("__mb_sync__".to_string()),
                        curve_name => FanSpeed::Curve(curve_name.to_string()),
                    };
                    if slot < 4 {
                        group.speeds[slot] = speed;
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_slot_pwm(move |dev_id, slot, percent| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(|| FanConfig {
                        speeds: vec![],
                        update_interval_ms: 500,
                    });
                    if let Some(group) = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id))
                    {
                        if slot < 4 {
                            group.speeds[slot] = FanSpeed::Constant(
                                ((percent as f32 / 100.0) * 255.0).round() as u8,
                            );
                        }
                    }
                }
            }
            // In-place update to avoid destroying the Slider during drag
            if let Some(w) = weak.upgrade() {
                let model = w.get_fan_groups();
                for i in 0..model.row_count() {
                    if let Some(group_data) = model.row_data(i) {
                        if group_data.device_id.as_str() == dev_id {
                            if let Some(mut slot_data) = group_data.slots.row_data(slot) {
                                slot_data.pwm_percent = percent;
                                group_data.slots.set_row_data(slot, slot_data);
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
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_fan_set_pwm_header(move |dev_id, slot, label| {
            let dev_id = dev_id.to_string();
            let slot = slot as usize;
            let label = label.to_string();
            let pwm_headers = lianli_shared::sensors::enumerate_pwm_headers();
            // Match label prefix (before the " (XX%)" suffix)
            let header_id = pwm_headers
                .iter()
                .find(|h| label.starts_with(&h.label))
                .map(|h| h.id.clone())
                .unwrap_or_default();
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let fc = c.fans.get_or_insert_with(|| FanConfig {
                        speeds: vec![],
                        update_interval_ms: 500,
                    });
                    if let Some(group) = fc
                        .speeds
                        .iter_mut()
                        .find(|g| g.device_id.as_deref() == Some(&dev_id))
                    {
                        if slot < 4 {
                            group.speeds[slot] = FanSpeed::Curve(format!(
                                "{}{}",
                                lianli_shared::fan::MB_SYNC_PREFIX,
                                header_id
                            ));
                        }
                    }
                }
            }
            refresh_fan_ui(&weak, &shared);
        });
    }
}

fn wire_lcd_callbacks(
    window: &MainWindow,
    shared: &Shared,
    editor: &editor::EditorHandle,
    browser: &template_browser::BrowserHandle,
) {
    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_add_lcd(move || {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    c.lcds.push(lianli_shared::config::LcdConfig {
                        index: None,
                        serial: None,
                        media_type: lianli_shared::media::MediaType::Image,
                        path: None,
                        fps: Some(30.0),
                        update_interval_ms: None,
                        rgb: None,
                        orientation: 0.0,
                        sensor_source_1: SensorSourceConfig::CpuUsage,
                        sensor_source_2: SensorSourceConfig::MemUsage,
                        sensor: None,
                        doublegauge: None,
                        template_id: None,
                    });
                }
            }
            refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_remove_lcd(move |idx| {
            {
                let mut state = shared.lock().unwrap();
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if idx < c.lcds.len() {
                        c.lcds.remove(idx);
                    }
                }
            }
            refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_update_lcd_field(move |idx, field, val| {
            let field_str = field.to_string();
            // Only rebuild UI for dropdown/button fields that affect layout.
            // Text fields update in-place in the LineEdit — rebuilding would steal focus.
            let needs_refresh = matches!(
                field_str.as_str(),
                "device"
                    | "media_type"
                    | "orientation"
                    | "sensor_source"
                    | "template_label"
                    | "template_id"
            ) || field_str == "gauge_range_add"
                || field_str == "gauge_range_remove";
            {
                let mut state = shared.lock().unwrap();
                let devices = state.devices.clone();
                let templates_snapshot = state.lcd_templates.clone();
                let resolved_sensor_source: Option<lianli_shared::media::SensorSourceConfig> = {
                    let val_str = val.to_string();
                    let sensor_idx: usize = val_str
                        .split('.')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let is_sensor_picker = field_str == "sensor_source";
                    if is_sensor_picker && !val_str.ends_with("Custom command") && sensor_idx > 0 {
                        state.available_sensors.get(sensor_idx - 1).map(|sensor| {
                            match &sensor.source {
                                lianli_shared::sensors::SensorSource::Hwmon {
                                    name,
                                    label,
                                    device_path,
                                } => lianli_shared::media::SensorSourceConfig::Hwmon {
                                    name: name.clone(),
                                    label: label.clone(),
                                    device_path: device_path.clone(),
                                },
                                lianli_shared::sensors::SensorSource::NvidiaGpu {
                                    gpu_index,
                                    metric,
                                } => lianli_shared::media::SensorSourceConfig::NvidiaGpu {
                                    gpu_index: *gpu_index,
                                    metric: *metric,
                                },
                                lianli_shared::sensors::SensorSource::AmdGpuUsage {
                                    card_index,
                                } => lianli_shared::media::SensorSourceConfig::AmdGpuUsage {
                                    card_index: *card_index,
                                },
                                lianli_shared::sensors::SensorSource::Command { cmd } => {
                                    lianli_shared::media::SensorSourceConfig::Command {
                                        cmd: cmd.clone(),
                                    }
                                }
                                lianli_shared::sensors::SensorSource::WirelessCoolant {
                                    device_id,
                                } => lianli_shared::media::SensorSourceConfig::WirelessCoolant {
                                    device_id: device_id.clone(),
                                },
                                lianli_shared::sensors::SensorSource::CpuUsage => {
                                    lianli_shared::media::SensorSourceConfig::CpuUsage
                                }
                                lianli_shared::sensors::SensorSource::MemUsage => {
                                    lianli_shared::media::SensorSourceConfig::MemUsage
                                }
                                lianli_shared::sensors::SensorSource::MemUsed => {
                                    lianli_shared::media::SensorSourceConfig::MemUsed
                                }
                                lianli_shared::sensors::SensorSource::MemFree => {
                                    lianli_shared::media::SensorSourceConfig::MemFree
                                }
                            }
                        })
                    } else {
                        None
                    }
                };
                if let Some(ref mut c) = state.config {
                    let idx = idx as usize;
                    if let Some(lcd) = c.lcds.get_mut(idx) {
                        let val = val.to_string();
                        match field_str.as_str() {
                            "device" => {
                                // Resolve label back to serial
                                let serial = conversions::lcd_label_to_serial(&val, &devices);
                                lcd.serial = Some(serial);
                            }
                            "media_type" => {
                                lcd.media_type = match val.as_str() {
                                    "Image" => lianli_shared::media::MediaType::Image,
                                    "Video" => lianli_shared::media::MediaType::Video,
                                    "GIF" => lianli_shared::media::MediaType::Gif,
                                    "Solid Color" => {
                                        lcd.rgb.get_or_insert([0, 0, 0]);
                                        lianli_shared::media::MediaType::Color
                                    }
                                    "Sensor Gauge" => {
                                        lcd.sensor.get_or_insert_with(default_sensor);
                                        lcd.path = None;
                                        lianli_shared::media::MediaType::Sensor
                                    }
                                    "Custom" => {
                                        lcd.path = None;
                                        lianli_shared::media::MediaType::Custom
                                    }
                                    _ => lcd.media_type,
                                };
                            }
                            "path" => lcd.path = Some(std::path::PathBuf::from(val)),
                            "orientation" => lcd.orientation = val.parse().unwrap_or(0.0),
                            "template_label" => {
                                // Resolve label → id via the snapshot taken
                                // before the mutable borrow of `state.config`.
                                if let Some(id) =
                                    conversions::template_id_for_label(&val, &templates_snapshot)
                                {
                                    lcd.template_id = Some(id);
                                }
                            }
                            "template_id" => {
                                lcd.template_id = Some(val);
                            }
                            "sensor_label" => {
                                lcd.sensor.get_or_insert_with(default_sensor).label = val;
                            }
                            "sensor_unit" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit = val;
                            }
                            "sensor_source" => {
                                let sensor_cfg = lcd.sensor.get_or_insert_with(default_sensor);
                                sensor_cfg.source = resolved_sensor_source.clone().unwrap_or(
                                    lianli_shared::media::SensorSourceConfig::Command {
                                        cmd: String::new(),
                                    },
                                );
                            }
                            "sensor_command" => {
                                lcd.sensor.get_or_insert_with(default_sensor).source =
                                    lianli_shared::media::SensorSourceConfig::Command { cmd: val };
                            }
                            "sensor_font_path" => {
                                lcd.sensor.get_or_insert_with(default_sensor).font_path =
                                    Some(std::path::PathBuf::from(val));
                            }
                            "sensor_font_name" => {
                                lcd.sensor.get_or_insert_with(default_sensor).font_path =
                                    lianli_shared::fonts::font_path_for_label(&val);
                            }
                            "fps" => lcd.fps = Some(val.parse::<f32>().unwrap_or(30.0)),
                            "rgb_r" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[0] = val.parse().unwrap_or(0)
                            }
                            "rgb_g" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[1] = val.parse().unwrap_or(0)
                            }
                            "rgb_b" => {
                                lcd.rgb.get_or_insert([0, 0, 0])[2] = val.parse().unwrap_or(0)
                            }
                            "sensor_decimal_places" => {
                                lcd.sensor.get_or_insert_with(default_sensor).decimal_places =
                                    val.parse().unwrap_or(0);
                            }
                            "update_interval_ms" => {
                                lcd.update_interval_ms =
                                    Some(val.parse().unwrap_or(1000).clamp(100, 10_000));
                            }
                            "sensor_value_font_size" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .value_font_size = val.parse().unwrap_or(120.0);
                            }
                            "sensor_unit_font_size" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit_font_size =
                                    val.parse().unwrap_or(40.0);
                            }
                            "sensor_label_font_size" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .label_font_size = val.parse().unwrap_or(30.0);
                            }
                            "sensor_start_angle" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_start_angle = val.parse().unwrap_or(135.0);
                            }
                            "sensor_sweep_angle" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_sweep_angle = val.parse().unwrap_or(270.0);
                            }
                            "sensor_outer_radius" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_outer_radius = val.parse().unwrap_or(200.0);
                            }
                            "sensor_thickness" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_thickness = val.parse().unwrap_or(30.0);
                            }
                            "sensor_corner_radius" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .bar_corner_radius = val.parse().unwrap_or(5.0);
                            }
                            "sensor_value_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).value_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_unit_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).unit_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_label_offset" => {
                                lcd.sensor.get_or_insert_with(default_sensor).label_offset =
                                    val.parse().unwrap_or(0);
                            }
                            "sensor_text_color_r" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[0] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_text_color_g" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[1] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_text_color_b" => {
                                lcd.sensor.get_or_insert_with(default_sensor).text_color[2] =
                                    val.parse().unwrap_or(255)
                            }
                            "sensor_bg_color_r" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[0] = val.parse().unwrap_or(0)
                            }
                            "sensor_bg_color_g" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[1] = val.parse().unwrap_or(0)
                            }
                            "sensor_bg_color_b" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .background_color[2] = val.parse().unwrap_or(0)
                            }
                            "sensor_gauge_bg_r" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[0] = val.parse().unwrap_or(40)
                            }
                            "sensor_gauge_bg_g" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[1] = val.parse().unwrap_or(40)
                            }
                            "sensor_gauge_bg_b" => {
                                lcd.sensor
                                    .get_or_insert_with(default_sensor)
                                    .gauge_background_color[2] = val.parse().unwrap_or(40)
                            }
                            "gauge_range_add" => {
                                let s = lcd.sensor.get_or_insert_with(default_sensor);
                                s.gauge_ranges.push(lianli_shared::media::SensorRange {
                                    max: Some(100.0),
                                    color: [0, 200, 0],
                                    alpha: 255,
                                });
                            }

                            f if f.starts_with("gauge_range_remove") => {
                                if let Ok(ridx) = val.parse::<usize>() {
                                    let s = lcd.sensor.get_or_insert_with(default_sensor);
                                    if ridx < s.gauge_ranges.len() {
                                        s.gauge_ranges.remove(ridx);
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_max_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_max_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<f32>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.max = Some(v);
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_r_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_r_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[0] = v;
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_g_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_g_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[1] = v;
                                        }
                                    }
                                }
                            }
                            f if f.starts_with("gauge_range_b_") => {
                                if let Some(ridx_str) = f.strip_prefix("gauge_range_b_") {
                                    if let (Ok(ridx), Ok(v)) =
                                        (ridx_str.parse::<usize>(), val.parse::<u8>())
                                    {
                                        let s = lcd.sensor.get_or_insert_with(default_sensor);
                                        if let Some(r) = s.gauge_ranges.get_mut(ridx) {
                                            r.color[2] = v;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if needs_refresh {
                refresh_lcd_ui(&weak, &shared);
            } else if let Some(w) = weak.upgrade() {
                w.set_config_dirty(true);
            }
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_pick_lcd_file(move |idx| {
            let shared2 = shared.clone();
            let weak2 = weak.clone();
            let idx = idx as usize;
            std::thread::spawn(move || {
                let is_sensor = {
                    let state = shared2.lock().unwrap();
                    state
                        .config
                        .as_ref()
                        .and_then(|c| c.lcds.get(idx))
                        .map(|lcd| lcd.media_type == lianli_shared::media::MediaType::Sensor)
                        .unwrap_or(false)
                };
                let mut dialog = rfd::FileDialog::new();
                dialog = if is_sensor {
                    dialog.add_filter("Images", &["jpg", "jpeg", "png", "bmp"])
                } else {
                    dialog.add_filter(
                        "Media",
                        &[
                            "jpg", "jpeg", "png", "bmp", "gif", "mp4", "avi", "mkv", "webm",
                        ],
                    )
                };
                let file = dialog.pick_file();
                if let Some(path) = file {
                    {
                        let mut state = shared2.lock().unwrap();
                        if let Some(ref mut c) = state.config {
                            if let Some(lcd) = c.lcds.get_mut(idx) {
                                lcd.path = Some(path);
                            }
                        }
                    }
                    refresh_lcd_ui(&weak2, &shared2);
                }
            });
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        let editor_window = editor.window.clone_strong();
        let editor_state = editor.state.clone();
        window.on_lcd_create_template(move |idx| {
            let handle = editor::EditorHandle {
                window: editor_window.clone_strong(),
                state: editor_state.clone(),
            };
            editor::open(&handle, &shared, idx as usize, None);
            refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_lcd_duplicate_template(move |idx| {
            duplicate_current_template(&shared, idx as usize);
            refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        window.on_lcd_delete_template(move |idx| {
            delete_current_template(&shared, idx as usize);
            refresh_lcd_ui(&weak, &shared);
        });
    }

    {
        let shared = shared.clone();
        let weak = window.as_weak();
        let editor_window = editor.window.clone_strong();
        let editor_state = editor.state.clone();
        window.on_lcd_edit_template(move |idx| {
            let (starting_template, target_idx) = {
                let state = shared.lock().unwrap();
                let lcd = state.config.as_ref().and_then(|c| c.lcds.get(idx as usize));
                let current_id = lcd.and_then(|l| l.template_id.clone());
                let source = current_id
                    .as_ref()
                    .and_then(|id| state.lcd_templates.iter().find(|t| &t.id == id).cloned());
                (source, idx as usize)
            };

            let handle = editor::EditorHandle {
                window: editor_window.clone_strong(),
                state: editor_state.clone(),
            };
            editor::open(&handle, &shared, target_idx, starting_template);
            let _ = weak;
        });
    }

    {
        let shared = shared.clone();
        let browser_window = browser.window.clone_strong();
        let browser_catalog = browser.catalog.clone();
        window.on_lcd_browse_templates(move || {
            let handle = template_browser::BrowserHandle {
                window: browser_window.clone_strong(),
                catalog: browser_catalog.clone(),
            };
            template_browser::open(&handle, &shared);
        });
    }
}

pub(crate) fn generate_template_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{:x}", nanos)
}

pub(crate) fn make_blank_template() -> lianli_shared::template::LcdTemplate {
    lianli_shared::template::LcdTemplate {
        id: generate_template_id("user"),
        name: "New Template".to_string(),
        base_width: 480,
        base_height: 480,
        background: lianli_shared::template::TemplateBackground::Color {
            rgb: [0, 0, 0, 255],
        },
        widgets: Vec::new(),
        rotated: false,
        target_device: None,
    }
}

fn duplicate_current_template(shared: &Shared, idx: usize) {
    let user_list: Option<Vec<lianli_shared::template::LcdTemplate>> = {
        let mut state = shared.lock().unwrap();
        let current_id = state
            .config
            .as_ref()
            .and_then(|c| c.lcds.get(idx))
            .and_then(|lcd| lcd.template_id.clone());
        let source = current_id
            .as_ref()
            .and_then(|id| state.lcd_templates.iter().find(|t| &t.id == id).cloned());
        if let Some(source) = source {
            let mut copy = source.clone();
            copy.id = generate_template_id("user");
            copy.name = next_unique_name(&source.name, &state.lcd_templates);
            let new_id = copy.id.clone();
            state.lcd_templates.push(copy);
            if let Some(ref mut c) = state.config {
                if let Some(lcd) = c.lcds.get_mut(idx) {
                    lcd.template_id = Some(new_id);
                }
            }
            Some(user_templates_only(&state.lcd_templates))
        } else {
            None
        }
    };
    if let Some(list) = user_list {
        send_set_templates(list);
    }
}

/// Generates a non-conflicting template name. If `base` is already a "(Copy N)"
/// form we strip the suffix before bumping, so duplicating "Foo (Copy 2)" yields
/// "Foo (Copy 3)" rather than "Foo (Copy 2) (Copy)".
pub(crate) fn next_unique_name(
    base: &str,
    existing: &[lianli_shared::template::LcdTemplate],
) -> String {
    let stem = strip_copy_suffix(base);
    let names: std::collections::HashSet<&str> = existing.iter().map(|t| t.name.as_str()).collect();
    if !names.contains(stem) && stem != base {
        return stem.to_string();
    }
    let first = format!("{stem} (Copy)");
    if !names.contains(first.as_str()) {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{stem} (Copy {i})");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    format!("{stem} (Copy {})", generate_template_id(""))
}

fn strip_copy_suffix(name: &str) -> &str {
    if let Some(idx) = name.rfind(" (Copy") {
        let tail = &name[idx + 6..];
        if tail == ")" || (tail.starts_with(' ') && tail.ends_with(')')) {
            return &name[..idx];
        }
    }
    name
}

pub(crate) fn next_unique_downloaded_name(
    base: &str,
    existing: &[lianli_shared::template::LcdTemplate],
) -> String {
    let names: std::collections::HashSet<&str> = existing.iter().map(|t| t.name.as_str()).collect();
    let first = format!("{base} (Downloaded)");
    if !names.contains(first.as_str()) {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{base} (Downloaded {i})");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    format!("{base} (Downloaded {})", generate_template_id(""))
}

fn delete_current_template(shared: &Shared, idx: usize) {
    let user_list = {
        let mut state = shared.lock().unwrap();
        let target_id = state
            .config
            .as_ref()
            .and_then(|c| c.lcds.get(idx))
            .and_then(|lcd| lcd.template_id.clone());
        let Some(target_id) = target_id else {
            return;
        };
        state.lcd_templates.retain(|t| t.id != target_id);
        if let Some(ref mut c) = state.config {
            for lcd in c.lcds.iter_mut() {
                if lcd.template_id.as_deref() == Some(target_id.as_str()) {
                    lcd.template_id = None;
                }
            }
        }
        user_templates_only(&state.lcd_templates)
    };
    send_set_templates(user_list);
}

pub(crate) fn user_templates_only(
    all: &[lianli_shared::template::LcdTemplate],
) -> Vec<lianli_shared::template::LcdTemplate> {
    all.to_vec()
}

pub(crate) fn send_set_templates(templates: Vec<lianli_shared::template::LcdTemplate>) {
    match ipc_client::send_request(&lianli_shared::ipc::IpcRequest::SetLcdTemplates { templates }) {
        Ok(lianli_shared::ipc::IpcResponse::Error { message }) => {
            tracing::warn!("SetLcdTemplates failed: {message}");
        }
        Err(e) => tracing::warn!("SetLcdTemplates IPC error: {e}"),
        _ => {}
    }
}

// ── Refresh helpers ──
// These read from SharedState (lock briefly), then push models to UI via invoke_from_event_loop.

fn refresh_fan_ui(weak: &slint::Weak<MainWindow>, shared: &Shared) {
    let (curves, fans, devices, sensors) = {
        let state = shared.lock().unwrap();
        let config = match state.config.as_ref() {
            Some(c) => c,
            None => return,
        };
        (
            config.fan_curves.clone(),
            config.fans.clone(),
            state.devices.clone(),
            state.available_sensors.clone(),
        )
    };

    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            w.set_fan_curves(conversions::fan_curves_to_model(&curves, &sensors));
            w.set_curve_names(conversions::curve_names_to_model(&curves));
            w.set_fan_speed_options(conversions::speed_options_model(&curves, true));
            w.set_config_dirty(true);
            let fc = fans.unwrap_or_default();
            let pwm_headers = lianli_shared::sensors::enumerate_pwm_headers();
            w.set_fan_groups(conversions::fan_groups_to_model(&fc, &devices, &pwm_headers));
        }
    })
    .ok();
}

pub(crate) fn refresh_lcd_ui(weak: &slint::Weak<MainWindow>, shared: &Shared) {
    let (lcds, devices, sensors, templates) = {
        let state = shared.lock().unwrap();
        match state.config.as_ref() {
            Some(c) => (
                c.lcds.clone(),
                state.devices.clone(),
                state.available_sensors.clone(),
                state.lcd_templates.clone(),
            ),
            None => return,
        }
    };

    let weak = weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            w.set_lcd_entries(conversions::lcd_entries_to_model(
                &lcds, &devices, &sensors, &templates,
            ));
            w.set_lcd_template_labels(conversions::template_labels_model(&templates));
            w.set_config_dirty(true);
        }
    })
    .ok();
}

fn default_sensor() -> lianli_shared::media::SensorDescriptor {
    lianli_shared::media::SensorDescriptor {
        label: "CPU".to_string(),
        unit: "\u{00B0}C".to_string(),
        source: lianli_shared::media::SensorSourceConfig::Command { cmd: String::new() },
        text_color: [255, 255, 255],
        background_color: [0, 0, 0],
        gauge_background_color: [40, 40, 40],
        gauge_ranges: vec![],
        update_interval_ms: 0, // legacy field, see SensorDescriptor docs
        gauge_start_angle: 135.0,
        gauge_sweep_angle: 270.0,
        gauge_outer_radius: 200.0,
        gauge_thickness: 30.0,
        bar_corner_radius: 5.0,
        value_font_size: 120.0,
        unit_font_size: 40.0,
        label_font_size: 30.0,
        font_path: None,
        decimal_places: 0,
        value_offset: 0,
        unit_offset: 0,
        label_offset: 0,
    }
}

/// Get or update an RGB zone's effect in the shared state, returning the updated effect.
fn with_zone_effect(
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
fn device_group_zone_count(shared: &Shared, dev_id: &str) -> Option<usize> {
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
fn send_rgb_effect(
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

fn get_or_create_device_config<'a>(
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

fn get_or_create_zone_config(dev: &mut RgbDeviceConfig, zone: u8) -> &mut RgbZoneConfig {
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
fn update_rgb_zone_in_place(
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
fn update_rgb_zone_colors_in_place(
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

fn parse_rgb_mode(s: &str) -> RgbMode {
    match s {
        "Off" => RgbMode::Off,
        "Direct" => RgbMode::Direct,
        "Static" => RgbMode::Static,
        "Rainbow" => RgbMode::Rainbow,
        "RainbowMorph" => RgbMode::RainbowMorph,
        "Breathing" => RgbMode::Breathing,
        "Runway" => RgbMode::Runway,
        "Meteor" => RgbMode::Meteor,
        "ColorCycle" => RgbMode::ColorCycle,
        "Staggered" => RgbMode::Staggered,
        "Tide" => RgbMode::Tide,
        "Mixing" => RgbMode::Mixing,
        "Voice" => RgbMode::Voice,
        "Door" => RgbMode::Door,
        "Render" => RgbMode::Render,
        "Ripple" => RgbMode::Ripple,
        "Reflect" => RgbMode::Reflect,
        "TailChasing" => RgbMode::TailChasing,
        "Paint" => RgbMode::Paint,
        "PingPong" => RgbMode::PingPong,
        "Stack" => RgbMode::Stack,
        "CoverCycle" => RgbMode::CoverCycle,
        "Wave" => RgbMode::Wave,
        "Racing" => RgbMode::Racing,
        "Lottery" => RgbMode::Lottery,
        "Intertwine" => RgbMode::Intertwine,
        "MeteorShower" => RgbMode::MeteorShower,
        "Collide" => RgbMode::Collide,
        "ElectricCurrent" => RgbMode::ElectricCurrent,
        "Kaleidoscope" => RgbMode::Kaleidoscope,
        "BigBang" => RgbMode::BigBang,
        "Vortex" => RgbMode::Vortex,
        "Pump" => RgbMode::Pump,
        "ColorsMorph" => RgbMode::ColorsMorph,
        _ => RgbMode::Off,
    }
}

fn parse_rgb_direction(s: &str) -> RgbDirection {
    match s {
        "Clockwise" => RgbDirection::Clockwise,
        "CounterClockwise" => RgbDirection::CounterClockwise,
        "Up" => RgbDirection::Up,
        "Down" => RgbDirection::Down,
        "Spread" => RgbDirection::Spread,
        "Gather" => RgbDirection::Gather,
        _ => RgbDirection::Clockwise,
    }
}

fn parse_rgb_scope(s: &str) -> RgbScope {
    match s {
        "All" => RgbScope::All,
        "Top" => RgbScope::Top,
        "Bottom" => RgbScope::Bottom,
        "Inner" => RgbScope::Inner,
        "Outer" => RgbScope::Outer,
        _ => RgbScope::All,
    }
}
