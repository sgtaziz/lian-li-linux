use lianli_shared::config::AppConfig;
use lianli_shared::ipc::DeviceInfo;
use lianli_shared::rgb::{RgbDeviceCapabilities, RgbMode, RgbScope};
use slint::{ModelRc, SharedString, VecModel};

pub(super) fn rgb_mode_to_string(mode: &RgbMode) -> String {
    format!("{mode:?}")
}

fn is_empty_fan_port(cap: &RgbDeviceCapabilities, devices: &[DeviceInfo]) -> bool {
    let Some((base, port)) = cap
        .device_id
        .rsplit_once(":group")
        .or_else(|| cap.device_id.rsplit_once(":port"))
    else {
        return false;
    };
    let port_device_id = format!("{base}:port{port}");
    devices
        .iter()
        .find(|d| d.device_id == port_device_id)
        .map(|d| d.fan_count == Some(0))
        .unwrap_or(false)
}

pub fn rgb_devices_to_model(
    capabilities: &[RgbDeviceCapabilities],
    config: &AppConfig,
    presets: &[lianli_shared::rgb::RgbPreset],
    devices: &[DeviceInfo],
) -> ModelRc<crate::RgbDeviceData> {
    let rgb_config = config.rgb.as_ref();
    let device_configs = rgb_config.map(|r| &r.devices);

    let items: Vec<crate::RgbDeviceData> = capabilities
        .iter()
        .filter(|cap| !is_empty_fan_port(cap, devices))
        .map(|cap| {
            let dev_cfg =
                device_configs.and_then(|devs| devs.iter().find(|d| d.device_id == cap.device_id));

            let mb_rgb_sync = dev_cfg.map(|d| d.mb_rgb_sync).unwrap_or(false);

            // Determine if device has group zones (scoped: Top/Bottom or Inner/Outer)
            let has_group_zones = cap.supported_scopes.iter().any(|scopes| {
                scopes.iter().any(|s| {
                    matches!(
                        s,
                        RgbScope::Top | RgbScope::Bottom | RgbScope::Inner | RgbScope::Outer
                    )
                })
            });

            // Check zone 0 config to determine synced state
            let z0_cfg = dev_cfg.and_then(|d| d.zones.iter().find(|z| z.zone_index == 0));
            let synced = if has_group_zones {
                if let Some(zcfg) = z0_cfg {
                    let is_per_fan = matches!(
                        zcfg.effect.mode,
                        RgbMode::Off | RgbMode::Static | RgbMode::Direct
                    ) && matches!(zcfg.effect.scope, RgbScope::All);
                    !is_per_fan
                } else {
                    false
                }
            } else {
                false
            };

            let zones: Vec<crate::RgbZoneData> = cap
                .zones
                .iter()
                .enumerate()
                .map(|(zidx, zone_info)| {
                    let zone_cfg =
                        dev_cfg.and_then(|d| d.zones.iter().find(|z| z.zone_index == zidx as u8));

                    let (mode, colors, speed, brightness, direction, scope, swap_lr, swap_tb) =
                        if let Some(zcfg) = zone_cfg {
                            let e = &zcfg.effect;
                            let colors: Vec<crate::RgbColorData> = e
                                .colors
                                .iter()
                                .map(|c| crate::RgbColorData {
                                    r: c[0] as i32,
                                    g: c[1] as i32,
                                    b: c[2] as i32,
                                })
                                .collect();
                            (
                                rgb_mode_to_string(&e.mode),
                                colors,
                                e.speed as i32,
                                e.brightness as i32,
                                format!("{:?}", e.direction),
                                format!("{:?}", e.scope),
                                zcfg.swap_lr,
                                zcfg.swap_tb,
                            )
                        } else {
                            (
                                "Off".to_string(),
                                vec![crate::RgbColorData { r: 255, g: 0, b: 0 }],
                                2,
                                3,
                                "Clockwise".to_string(),
                                "All".to_string(),
                                false,
                                false,
                            )
                        };

                    let led_colors: Vec<crate::RgbColorData> = if mode == "Direct" {
                        let base = colors.first().cloned().unwrap_or(crate::RgbColorData {
                            r: 0,
                            g: 0,
                            b: 0,
                        });
                        vec![base; zone_info.led_count as usize]
                    } else {
                        Vec::new()
                    };

                    crate::RgbZoneData {
                        zone_index: zidx as i32,
                        zone_name: SharedString::from(&zone_info.name),
                        led_count: zone_info.led_count as i32,
                        mode: SharedString::from(&mode),
                        colors: ModelRc::new(VecModel::from(colors)),
                        led_colors: ModelRc::new(VecModel::from(led_colors)),
                        speed,
                        brightness,
                        direction: SharedString::from(&direction),
                        scope: SharedString::from(&scope),
                        swap_lr,
                        swap_tb,
                        is_synced_zone: synced && zidx != 0,
                    }
                })
                .collect();

            let supported_modes: Vec<SharedString> = cap
                .supported_modes
                .iter()
                .map(|m| SharedString::from(rgb_mode_to_string(m)))
                .collect();

            // Flatten all scopes across zones into a unique set
            let mut all_scopes: Vec<String> = cap
                .supported_scopes
                .iter()
                .flat_map(|s| s.iter().map(|sc| format!("{sc:?}")))
                .collect();
            all_scopes.sort();
            all_scopes.dedup();
            let supported_scopes: Vec<SharedString> = all_scopes
                .iter()
                .map(|s| SharedString::from(s.as_str()))
                .collect();

            let preset_names: Vec<SharedString> = presets
                .iter()
                .filter(|p| p.device_id == cap.device_id)
                .map(|p| SharedString::from(&p.name))
                .collect();

            let active_preset = dev_cfg
                .and_then(|d| d.active_preset.as_deref())
                .unwrap_or("");

            crate::RgbDeviceData {
                device_id: SharedString::from(&cap.device_id),
                device_name: SharedString::from(&cap.device_name),
                total_leds: cap.total_led_count as i32,
                mb_rgb_sync,
                supports_mb_sync: cap.supports_mb_rgb_sync,
                supports_direction: cap.supports_direction,
                has_group_zones,
                synced,
                is_wireless: cap.device_id.starts_with("wireless:"),
                active_preset: SharedString::from(active_preset),
                supported_modes: ModelRc::new(VecModel::from(supported_modes)),
                supported_scopes: ModelRc::new(VecModel::from(supported_scopes)),
                preset_names: ModelRc::new(VecModel::from(preset_names)),
                zones: ModelRc::new(VecModel::from(zones)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}
