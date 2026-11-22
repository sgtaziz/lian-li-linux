use super::ServiceManager;
use crate::persistence;
use lianli_devices::detect::DetectedDevice;
use lianli_shared::config::AppConfig;
use lianli_shared::device_id::DeviceFamily;
use std::collections::{HashMap, HashSet};

struct Identity {
    family: DeviceFamily,
    pid: u16,
    old: String,
    current: String,
    serial: Option<String>,
}

impl From<&DetectedDevice> for Identity {
    fn from(device: &DetectedDevice) -> Self {
        Self {
            family: device.family,
            pid: device.pid,
            old: device.legacy_device_id(),
            current: device.device_id(),
            serial: device.serial.clone(),
        }
    }
}

fn migrate(config: &mut AppConfig, devices: &[Identity]) -> (bool, Vec<String>) {
    let mut changed = false;
    let mut aliases = HashMap::new();
    let mut warnings = Vec::new();
    for device in devices {
        if let Some(serial) = device
            .serial
            .as_ref()
            .filter(|_| device.family == DeviceFamily::Ene6k77)
        {
            if config.ene6k77.contains_key(serial) && !config.ene6k77.contains_key(&device.current)
            {
                let owners = devices
                    .iter()
                    .filter(|d| {
                        d.family == DeviceFamily::Ene6k77 && d.serial.as_ref() == Some(serial)
                    })
                    .count();
                if owners == 1 {
                    let quantities = config.ene6k77.remove(serial).unwrap();
                    config.ene6k77.insert(device.current.clone(), quantities);
                    changed = true;
                } else {
                    warnings.push(format!("ENE fan quantities for serial {serial} have multiple possible owners. Reassign quantities to each connected hub; the old settings are retained."));
                }
            }
        }
        if device.old == device.current {
            continue;
        }
        if devices.iter().filter(|d| d.old == device.old).count() != 1 {
            if legacy_controls_exist(config, &device.old) {
                warnings.push(format!("Legacy device ID {} is shared. Its saved fan and RGB settings are retained but cannot be assigned automatically. Configure each controller using its USB port identity.", device.old));
            }
            continue;
        }
        aliases.insert(device.old.clone(), device.current.clone());
        for port in 0..4 {
            aliases.insert(
                format!("{}:port{port}", device.old),
                format!("{}:port{port}", device.current),
            );
            aliases.insert(
                format!("{}:group{port}", device.old),
                format!("{}:group{port}", device.current),
            );
        }
    }
    for device in devices.iter().filter(|d| d.family.has_lcd()) {
        let matches = |serial: &str, d: &Identity| {
            d.family.has_lcd() && (serial == d.old || d.serial.as_deref() == Some(serial))
        };
        let existing: HashSet<_> = config
            .lcds
            .iter()
            .filter_map(|lcd| lcd.serial.clone())
            .collect();
        for lcd in &mut config.lcds {
            if let Some(serial) = &mut lcd.serial {
                if serial == &device.current
                    || !matches(serial, device)
                    || existing.contains(&device.current)
                {
                    continue;
                }
                if devices.iter().filter(|d| matches(serial, d)).count() == 1 {
                    *serial = device.current.clone();
                    changed = true;
                } else {
                    warnings.push(format!("LCD settings for {serial} have multiple possible owners. Select the intended display using its USB port identity; the old settings are retained."));
                }
            }
        }
    }
    for device in devices.iter().filter(|d| d.family.has_pump()) {
        if device.old == device.current
            || config.aio.contains_key(&device.current)
            || !config.aio.contains_key(&device.old)
        {
            continue;
        }
        if devices
            .iter()
            .filter(|d| d.family.has_pump() && d.old == device.old)
            .count()
            == 1
        {
            if let Some(aio) = config.aio.remove(&device.old) {
                config.aio.insert(device.current.clone(), aio);
                changed = true;
            }
        } else {
            warnings.push(format!("Pump settings for {} have multiple possible owners. Configure each AIO using its USB port identity; the old settings are retained.", device.old));
        }
    }
    if let Some(fans) = &mut config.fans {
        let existing: HashSet<_> = fans
            .speeds
            .iter()
            .filter_map(|g| g.device_id.clone())
            .collect();
        for group in &mut fans.speeds {
            if let Some(id) = &mut group.device_id {
                if let Some(new) = aliases.get(id).filter(|new| !existing.contains(*new)) {
                    *id = new.clone();
                    changed = true;
                }
            }
        }
    }
    if let Some(rgb) = &mut config.rgb {
        let existing: HashSet<_> = rgb.devices.iter().map(|d| d.device_id.clone()).collect();
        for device in &mut rgb.devices {
            if let Some(new) = aliases
                .get(&device.device_id)
                .filter(|new| !existing.contains(*new))
            {
                device.device_id = new.clone();
                changed = true;
            }
            if let Some(model) = devices.iter().find_map(|identity| {
                if identity.family != DeviceFamily::Ene6k77 {
                    return None;
                }
                let group = device
                    .device_id
                    .strip_prefix(&format!("{}:group", identity.current))?;
                if !matches!(group, "0" | "1" | "2" | "3") {
                    return None;
                }
                lianli_devices::ene6k77::Ene6k77Model::from_pid(identity.pid)
            }) {
                changed |= device.expand_legacy_group_zone(model.max_fans_per_group());
            }
        }
        if let Some(merge) = &mut rgb.merge_lighting {
            for list in [&mut merge.device_order, &mut merge.disabled_devices] {
                let existing: HashSet<_> = list.iter().cloned().collect();
                for id in list.iter_mut() {
                    if let Some(new) = aliases.get(id).filter(|new| !existing.contains(*new)) {
                        *id = new.clone();
                        changed = true;
                    }
                }
            }
        }
    }
    warnings.sort();
    warnings.dedup();
    (changed, warnings)
}

fn legacy_controls_exist(config: &AppConfig, old: &str) -> bool {
    let matches = |id: &str| {
        id == old
            || id
                .strip_prefix(old)
                .is_some_and(|suffix| suffix.starts_with(":port") || suffix.starts_with(":group"))
    };
    config.fans.as_ref().is_some_and(|fans| {
        fans.speeds
            .iter()
            .any(|group| group.device_id.as_deref().is_some_and(matches))
    }) || config.rgb.as_ref().is_some_and(|rgb| {
        rgb.devices.iter().any(|device| matches(&device.device_id))
            || rgb.merge_lighting.as_ref().is_some_and(|merge| {
                merge
                    .device_order
                    .iter()
                    .chain(&merge.disabled_devices)
                    .any(|id| matches(id))
            })
    })
}

impl ServiceManager {
    pub(super) fn migrate_wired_config(&mut self, devices: &[DetectedDevice]) -> bool {
        if self.startup_image_job.is_some() {
            self.startup_config_pending = true;
            return false;
        }
        let identities: Vec<_> = devices.iter().map(Identity::from).collect();
        let mut state = self.ipc.state.lock();
        let Some(original) = state.config.as_ref().or(self.config.as_ref()) else {
            return false;
        };
        let mut config = original.clone();
        let (changed, warnings) = migrate(&mut config, &identities);
        if changed {
            tracing::info!("Migrating wired device identities and legacy RGB group settings");
            match persistence::write_config(&self.config_path, &config) {
                Ok(()) => {
                    state.state_health.wired_identity_save_error(None);
                }
                Err(error) => state
                    .state_health
                    .wired_identity_save_error(Some(&format!("{error:#}"))),
            }
            self.config = Some(config.clone());
            state.config = Some(config);
        }
        state.state_health.wired_identity_warnings(&warnings);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::fan::{FanConfig, FanGroup, FanSpeed};
    use lianli_shared::rgb::{MergeLightingConfig, RgbAppConfig, RgbDeviceConfig};

    fn identity(port: u8) -> Identity {
        Identity {
            family: DeviceFamily::Ene6k77,
            pid: 0xa102,
            old: "hid:shared".into(),
            current: format!("hid:0cf2:a102:1-{port}"),
            serial: Some("shared".into()),
        }
    }

    fn config() -> AppConfig {
        let mut config = AppConfig {
            fans: Some(FanConfig {
                speeds: vec![FanGroup {
                    device_id: Some("hid:shared:port2".into()),
                    speeds: std::array::from_fn(|_| FanSpeed::Constant(70)),
                }],
                ..Default::default()
            }),
            rgb: Some(RgbAppConfig {
                devices: vec![RgbDeviceConfig {
                    device_id: "hid:shared:group2".into(),
                    fan_led_count: None,
                    mb_rgb_sync: true,
                    active_preset: None,
                    zones: vec![],
                    regions: None,
                    effect_memory: vec![],
                    port_merge: None,
                    reverse_port: false,
                }],
                merge_lighting: Some(MergeLightingConfig {
                    device_order: vec!["hid:shared:group2".into()],
                    disabled_devices: vec!["hid:shared:group1".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        config
            .ene6k77
            .entry("shared".into())
            .or_default()
            .fan_quantities
            .insert(2, 4);
        config
    }

    #[test]
    fn legacy_rgb_groups_expand_before_or_after_identity_migration() {
        for already_migrated in [false, true] {
            for pid in 0xa100..=0xa106 {
                let mut hub = identity(2);
                hub.pid = pid;
                hub.current = format!("hid:0cf2:{pid:04x}:1-2");
                let mut config = config();
                let rgb = config.rgb.as_mut().unwrap();
                let device = &mut rgb.devices[0];
                if already_migrated {
                    device.device_id = format!("{}:group2", hub.current);
                }
                device.zones = serde_json::from_value(serde_json::json!([{
                    "zone_index": 0,
                    "effect": {"mode": "Static", "colors": [[12,34,56]], "brightness": 2},
                    "swap_lr": true
                }]))
                .unwrap();
                let original = device.zones[0].clone();
                assert!(migrate(&mut config, &[hub]).0);
                let zones = &config.rgb.as_ref().unwrap().devices[0].zones;
                let slots = lianli_devices::ene6k77::Ene6k77Model::from_pid(pid)
                    .unwrap()
                    .max_fans_per_group();
                assert_eq!(zones.len(), usize::from(slots));
                for (index, zone) in zones.iter().enumerate() {
                    assert_eq!(zone.zone_index as usize, index);
                    assert_eq!(zone.effect, original.effect);
                    assert_eq!(zone.swap_lr, original.swap_lr);
                }
                let mut reloaded: AppConfig =
                    serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
                let mut hub = identity(2);
                hub.pid = pid;
                hub.current = format!("hid:0cf2:{pid:04x}:1-2");
                assert_eq!(migrate(&mut reloaded, &[hub]), (false, vec![]));
            }
        }
    }

    #[test]
    fn sparse_non_ene_rgb_and_offline_groups_are_not_expanded() {
        let mut config = config();
        config.rgb.as_mut().unwrap().devices[0].zones =
            serde_json::from_value(serde_json::json!([{
            "zone_index": 0, "effect": {"mode": "Static"}
            }]))
            .unwrap();
        let before = serde_json::to_value(&config).unwrap();
        assert!(!migrate(&mut config, &[]).0);
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
        let mut aio = identity(2);
        aio.family = DeviceFamily::Galahad2Lcd;
        aio.pid = 0x7395;
        aio.current = "hid:0416:7395:1-2".into();
        config.rgb.as_mut().unwrap().devices[0].device_id = aio.old.clone();
        assert!(migrate(&mut config, &[aio]).0);
        assert_eq!(config.rgb.unwrap().devices[0].zones.len(), 1);
    }

    #[test]
    fn migrates_unique_settings_and_round_trips_without_repeating() {
        let mut config = config();
        let (changed, warnings) = migrate(&mut config, &[identity(2)]);
        assert!(changed);
        assert!(warnings.is_empty());
        assert_eq!(
            config.fans.as_ref().unwrap().speeds[0].device_id.as_deref(),
            Some("hid:0cf2:a102:1-2:port2")
        );
        assert_eq!(
            config.fans.as_ref().unwrap().speeds[0].speeds[0],
            FanSpeed::Constant(70)
        );
        let rgb = config.rgb.as_ref().unwrap();
        assert_eq!(rgb.devices[0].device_id, "hid:0cf2:a102:1-2:group2");
        assert!(rgb.devices[0].mb_rgb_sync);
        assert_eq!(
            rgb.merge_lighting.as_ref().unwrap().device_order,
            ["hid:0cf2:a102:1-2:group2"]
        );
        assert_eq!(
            rgb.merge_lighting.as_ref().unwrap().disabled_devices,
            ["hid:0cf2:a102:1-2:group1"]
        );
        assert_eq!(config.ene6k77["hid:0cf2:a102:1-2"].fan_quantities[&2], 4);
        assert!(!config.ene6k77.contains_key("shared"));
        let mut reloaded = serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap();
        assert_eq!(migrate(&mut reloaded, &[identity(2)]), (false, vec![]));
    }

    #[test]
    fn duplicate_and_offline_legacy_settings_are_retained() {
        let mut config = config();
        let before = serde_json::to_value(&config).unwrap();
        let (changed, warnings) = migrate(&mut config, &[identity(2), identity(3)]);
        assert!(!changed);
        assert!(!warnings.is_empty());
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
        assert_eq!(migrate(&mut config, &[]), (false, vec![]));
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
    }

    #[test]
    fn explicit_physical_settings_win_without_deleting_legacy_settings() {
        let mut config = config();
        let fans = config.fans.as_mut().unwrap();
        fans.speeds.push(FanGroup {
            device_id: Some("hid:0cf2:a102:1-2:port2".into()),
            speeds: std::array::from_fn(|_| FanSpeed::Constant(90)),
        });
        config
            .ene6k77
            .entry("hid:0cf2:a102:1-2".into())
            .or_default()
            .fan_quantities
            .insert(2, 1);
        migrate(&mut config, &[identity(2)]);
        assert_eq!(
            config.fans.as_ref().unwrap().speeds[0].device_id.as_deref(),
            Some("hid:shared:port2")
        );
        assert_eq!(
            config.fans.as_ref().unwrap().speeds[1].speeds[0],
            FanSpeed::Constant(90)
        );
        assert_eq!(config.ene6k77["hid:0cf2:a102:1-2"].fan_quantities[&2], 1);
        assert_eq!(config.ene6k77["shared"].fan_quantities[&2], 4);
    }

    #[test]
    fn cross_family_collision_does_not_steal_fan_or_rgb_settings() {
        let mut config = config();
        let mut aio = identity(3);
        aio.family = DeviceFamily::HydroShiftLcd;
        aio.current = "hid:0416:7371:1-3".into();
        let (_, warnings) = migrate(&mut config, &[identity(2), aio]);
        assert!(!warnings.is_empty());
        assert_eq!(
            config.fans.unwrap().speeds[0].device_id.as_deref(),
            Some("hid:shared:port2")
        );
        assert_eq!(
            config.rgb.unwrap().devices[0].device_id,
            "hid:shared:group2"
        );
        assert!(config.ene6k77.contains_key("hid:0cf2:a102:1-2"));
    }

    #[test]
    fn all_wired_controller_families_migrate_fan_and_rgb_references() {
        for family in [
            DeviceFamily::Ene6k77,
            DeviceFamily::TlFan,
            DeviceFamily::StrimerPlus,
            DeviceFamily::Galahad2Trinity,
            DeviceFamily::HydroShiftLcd,
            DeviceFamily::WiredReceiver,
            DeviceFamily::UniversalScreenLighting,
        ] {
            let mut device = identity(2);
            device.family = family;
            let mut config = config();
            config.ene6k77.clear();
            let (changed, warnings) = migrate(&mut config, &[device]);
            assert!(changed, "{family:?}");
            assert!(warnings.is_empty(), "{family:?}");
            assert_eq!(
                config.fans.unwrap().speeds[0].device_id.as_deref(),
                Some("hid:0cf2:a102:1-2:port2")
            );
            assert_eq!(
                config.rgb.unwrap().devices[0].device_id,
                "hid:0cf2:a102:1-2:group2"
            );
        }
    }

    #[test]
    fn lcd_and_pump_settings_migrate_using_their_capabilities() {
        let mut config = AppConfig::default();
        config.lcds.push(serde_json::from_value(serde_json::json!({
            "serial": "shared", "type": "color", "rgb": [1, 2, 3], "brightness": 42, "orientation": 270
        })).unwrap());
        config.aio.insert(
            "hid:shared".into(),
            lianli_shared::aio::AioConfig::defaults_for_host(),
        );
        let original_aio = serde_json::to_value(&config.aio["hid:shared"]).unwrap();
        let mut lcd = identity(3);
        lcd.family = DeviceFamily::HydroShiftLcd;
        lcd.current = "hid:0416:7371:1-3".into();
        assert!(migrate(&mut config, &[identity(2), lcd]).0);
        assert_eq!(config.lcds[0].serial.as_deref(), Some("hid:0416:7371:1-3"));
        assert_eq!(config.lcds[0].brightness(), 42);
        assert_eq!(config.lcds[0].orientation, 270.0);
        assert_eq!(
            serde_json::to_value(&config.aio["hid:0416:7371:1-3"]).unwrap(),
            original_aio
        );
        assert!(!config.aio.contains_key("hid:shared"));
    }

    #[test]
    fn ambiguous_lcds_keep_their_media_and_physical_settings_win() {
        let mut config = AppConfig::default();
        config.lcds.push(
            serde_json::from_value(
                serde_json::json!({"serial":"hid:shared", "type":"color", "rgb":[1,2,3]}),
            )
            .unwrap(),
        );
        let mut first = identity(2);
        first.family = DeviceFamily::Slv3Lcd;
        let mut second = identity(3);
        second.family = DeviceFamily::Slv3Lcd;
        let before = serde_json::to_value(&config).unwrap();
        let (changed, warnings) = migrate(&mut config, &[first, second]);
        assert!(!changed);
        assert!(!warnings.is_empty());
        assert_eq!(serde_json::to_value(config).unwrap(), before);
    }
}
