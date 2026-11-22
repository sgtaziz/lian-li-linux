use super::*;
use anyhow::Context;

struct ConfiguredRender {
    state: RenderState,
    direct_colors: HashMap<u8, Vec<[u8; 3]>>,
}

impl RgbController {
    fn validate_fan_led_counts(&self, config: &RgbAppConfig) -> anyhow::Result<()> {
        for saved in &config.devices {
            if let Some(device) = self.wired.get(&saved.device_id) {
                if let Some(control) = device.fan_led_count_control() {
                    control
                        .resolve(saved.fan_led_count)
                        .map_err(anyhow::Error::msg)?;
                } else {
                    anyhow::ensure!(
                        saved.fan_led_count.is_none(),
                        "Adjustable fan LED count is unsupported for {}",
                        saved.device_id
                    );
                }
            }
        }
        Ok(())
    }

    pub fn validate_config(&self, config: &RgbAppConfig) -> anyhow::Result<()> {
        lianli_shared::rgb::validate_effect_memory(config).map_err(anyhow::Error::msg)?;
        self.validate_fan_led_counts(config)?;
        if !config.enabled || config.openrgb_server {
            return Ok(());
        }
        self.prepare_sync(config)?;
        let presets = self.presets.clone();
        for device in &config.devices {
            if !device.mb_rgb_sync {
                self.configured_group_effects(device, &presets)?;
            }
            if device.mb_rgb_sync || !self.software_controlled(&device.device_id) {
                continue;
            }
            let state = self.configured_render(device, &presets)?.state;
            if let Some(profile) = self.regional_profile(&device.device_id) {
                if let Some(regions) = &state.regions {
                    let animation = lianli_media::rgb::family::render(profile, regions)?;
                    if let Some(wireless_device) = self.wireless_state.get(&device.device_id) {
                        self.wireless
                            .as_ref()
                            .context("wireless RGB controller is unavailable")?
                            .prepare_rgb_animation(
                                &wireless_device.mac,
                                &animation.frames,
                                animation.timing(),
                            )?;
                    } else if let Some(wired) = self.wired.get(&device.device_id) {
                        wired.validate_software_animation(&animation.frames, animation.timing())?;
                    }
                    continue;
                }
            }
            if let Some(wireless_device) = self.wireless_state.get(&device.device_id) {
                self.wireless
                    .as_ref()
                    .context("wireless RGB controller is unavailable")?
                    .prepare_rgb_upload(&wireless_device.mac, &state.frames(), FRAME_INTERVAL_MS)?;
            } else if let Some(wired) = self.wired.get(&device.device_id) {
                wired.validate_software_animation(
                    &state.frames(),
                    lianli_shared::rgb::RgbPlaybackTiming::from_millis(FRAME_INTERVAL_MS),
                )?;
            }
        }
        Ok(())
    }

    pub fn apply_config(&mut self, config: &RgbAppConfig, presets: &[RgbPreset]) {
        if let Err(error) = self.validate_fan_led_counts(config) {
            warn!("Invalid fan LED counts: {error}");
            return;
        }
        for (id, device) in &self.wired {
            if device.fan_led_count_control().is_none() {
                continue;
            }
            let count = config
                .devices
                .iter()
                .find(|saved| &saved.device_id == id)
                .and_then(|saved| saved.fan_led_count);
            match device.configure_fan_led_count(count) {
                Ok(true) => {
                    self.capabilities_changed();
                    self.configured.remove(id);
                    self.mb_sync_state.remove(id);
                    self.applied.remove(id);
                    self.sync_signature = None;
                }
                Ok(false) => {}
                Err(error) => {
                    warn!("Invalid fan LED count for {id}: {error}");
                    return;
                }
            }
        }
        let previous = self.config.replace(config.clone());
        self.presets = presets.to_vec();
        self.openrgb_server_enabled = config.openrgb_server;
        if !config.enabled || self.is_openrgb_controlled() {
            self.clear_pending();
            return;
        }
        if self.thermal_override_active() {
            return;
        }
        if let Err(error) = self.apply_sync(config) {
            warn!("Failed to apply RGB synchronization: {error:#}");
        }

        let removed: Vec<_> = previous
            .iter()
            .flat_map(|config| &config.devices)
            .map(|device| &device.device_id)
            .filter(|id| !self.sync_active.contains(*id))
            .filter(|id| !config.devices.iter().any(|d| &d.device_id == *id))
            .cloned()
            .collect();
        for id in removed {
            self.clear_device_pending(&id);
            self.rendered.remove(&id);
            self.configured_direct_colors
                .retain(|(device, _), _| device != &id);
        }

        let mut ordered: Vec<_> = config.devices.iter().collect();
        ordered.sort_by_key(|device| self.is_short_strimer(&device.device_id));
        for device in ordered {
            if self.sync_active.contains(&device.device_id) {
                continue;
            }
            let result = (|| -> anyhow::Result<()> {
                let mb_rgb_sync = device.mb_rgb_sync
                    || device
                        .device_id
                        .rsplit_once(":group")
                        .is_some_and(|(base, _)| {
                            config.devices.iter().any(|sibling| {
                                sibling.mb_rgb_sync
                                    && sibling
                                        .device_id
                                        .rsplit_once(":group")
                                        .is_some_and(|(other, _)| other == base)
                            })
                        });
                let preset = device.active_preset.as_ref().and_then(|name| {
                    presets
                        .iter()
                        .find(|preset| &preset.name == name && preset.device_id == device.device_id)
                });
                let merge = self.port_merge(config, &device.device_id);
                let signature = serde_json::to_string(&(
                    mb_rgb_sync,
                    &merge,
                    device.reverse_port,
                    device.fan_led_count,
                    &device.zones,
                    &device.regions,
                    preset.map(|preset| (&preset.zones, &preset.regions)),
                ))?;
                if self.configured.get(&device.device_id) == Some(&signature) {
                    return Ok(());
                }
                self.configured.remove(&device.device_id);
                if merge == Some(None) && !mb_rgb_sync {
                    self.wired[&device.device_id].apply_port_merge(None)?;
                }
                if mb_rgb_sync {
                    self.set_mb_rgb_sync(&device.device_id, true)?;
                } else if let Some(Some(merge)) = &merge {
                    if self.is_merge_leader(&device.device_id) {
                        self.wired[&device.device_id].apply_port_merge(Some(merge))?;
                    }
                    self.apply_directions(device)?;
                } else if self.software_controlled(&device.device_id) {
                    let next = self.configured_render(device, presets)?;
                    self.apply_render(&device.device_id, next.state)?;
                    self.configured_direct_colors
                        .retain(|(id, _), _| id != &device.device_id);
                    self.configured_direct_colors.extend(
                        next.direct_colors
                            .into_iter()
                            .map(|(zone, colors)| ((device.device_id.clone(), zone), colors)),
                    );
                } else if let Some(effects) = self.configured_group_effects(device, presets)? {
                    self.set_mb_rgb_sync(&device.device_id, false)?;
                    self.wired[&device.device_id].set_group_effects(&effects)?;
                } else {
                    for zone in &device.zones {
                        self.set_effect(&device.device_id, zone.zone_index, &zone.effect)?;
                    }
                    self.apply_directions(device)?;
                }
                if mb_rgb_sync || !self.software_controlled(&device.device_id) {
                    self.configured_direct_colors
                        .retain(|(id, _), _| id != &device.device_id);
                }
                self.configured.insert(device.device_id.clone(), signature);
                Ok(())
            })();
            if let Err(error) = result {
                warn!(
                    "Failed to apply RGB config for {}: {error}",
                    device.device_id
                );
            }
        }
    }

    pub fn saved_group_effects(&self, id: &str) -> anyhow::Result<Vec<RgbEffect>> {
        let Some(device) = self
            .config
            .as_ref()
            .and_then(|config| config.devices.iter().find(|device| device.device_id == id))
        else {
            return Ok(Vec::new());
        };
        Ok(self
            .configured_group_effects(device, &self.presets)?
            .unwrap_or_default())
    }

    pub fn saved_zone_effects(&self, id: &str) -> Vec<(u8, RgbEffect)> {
        let Some(device) = self
            .config
            .as_ref()
            .and_then(|config| config.devices.iter().find(|device| device.device_id == id))
        else {
            return Vec::new();
        };
        if let Some(preset) = device.active_preset.as_ref().and_then(|name| {
            self.presets
                .iter()
                .find(|preset| &preset.name == name && preset.device_id == id)
        }) {
            return preset
                .zones
                .iter()
                .filter_map(|zone| {
                    let effect = if zone.colors.is_empty() {
                        zone.effect.clone()?
                    } else {
                        RgbEffect {
                            mode: RgbMode::Direct,
                            colors: zone.colors.clone(),
                            ..Default::default()
                        }
                    };
                    Some((zone.zone, effect))
                })
                .collect();
        }
        device
            .zones
            .iter()
            .map(|zone| (zone.zone_index, zone.effect.clone()))
            .collect()
    }

    fn configured_group_effects(
        &self,
        device: &lianli_shared::rgb::RgbDeviceConfig,
        presets: &[RgbPreset],
    ) -> anyhow::Result<Option<Vec<RgbEffect>>> {
        let Some(wired) = self
            .wired
            .get(&device.device_id)
            .filter(|wired| !wired.hardware_regions().is_empty())
        else {
            return Ok(None);
        };
        let mut effective = device.clone();
        if let Some(preset) = device.active_preset.as_ref().and_then(|name| {
            presets
                .iter()
                .find(|preset| &preset.name == name && preset.device_id == device.device_id)
        }) {
            effective.regions = preset.regions.clone();
            effective.zones = preset
                .zones
                .iter()
                .filter_map(|zone| {
                    zone.effect
                        .clone()
                        .map(|effect| lianli_shared::rgb::RgbZoneConfig {
                            zone_index: zone.zone,
                            effect,
                            swap_lr: false,
                            swap_tb: false,
                        })
                })
                .collect();
        }
        wired.resolve_group_config(&effective)
    }

    fn configured_render(
        &self,
        device: &lianli_shared::rgb::RgbDeviceConfig,
        presets: &[RgbPreset],
    ) -> anyhow::Result<ConfiguredRender> {
        let old = self.render_state(&device.device_id)?;
        let has_live_frame = self.rendered.contains_key(&device.device_id);
        let preset = device.active_preset.as_ref().and_then(|name| {
            presets
                .iter()
                .find(|preset| &preset.name == name && preset.device_id == device.device_id)
        });
        let mut effective = device.clone();
        if let Some(preset) = preset {
            effective.regions = preset.regions.clone();
            effective.zones = preset
                .zones
                .iter()
                .filter_map(|zone| {
                    let effect = if !zone.colors.is_empty() {
                        RgbEffect {
                            mode: RgbMode::Direct,
                            ..Default::default()
                        }
                    } else {
                        zone.effect.clone()?
                    };
                    Some(lianli_shared::rgb::RgbZoneConfig {
                        zone_index: zone.zone,
                        effect,
                        swap_lr: false,
                        swap_tb: false,
                    })
                })
                .collect();
        }
        let mut next = RenderState::new(old.counts.clone());
        effective
            .zones
            .retain(|zone| usize::from(zone.zone_index) < next.counts.len());
        if let Some(profile) = self.regional_profile(&device.device_id) {
            next.regions = regions::resolve(&effective, profile)?;
        } else {
            anyhow::ensure!(
                effective.regions.is_none(),
                "device does not support regional RGB effects"
            );
        }
        let mut direct_colors = HashMap::new();
        if next.regions.is_none() {
            for zone in &effective.zones {
                if zone.effect.mode == RgbMode::Direct {
                    let key = (device.device_id.clone(), zone.zone_index);
                    let colors = preset.and_then(|preset| {
                        preset
                            .zones
                            .iter()
                            .find(|entry| entry.zone == zone.zone_index && !entry.colors.is_empty())
                    });
                    if let Some(colors) = colors {
                        next.set_direct(zone.zone_index, &colors.colors)?;
                    } else if !zone.effect.colors.is_empty() {
                        let changed =
                            self.configured_direct_colors.get(&key) != Some(&zone.effect.colors);
                        direct_colors.insert(zone.zone_index, zone.effect.colors.clone());
                        if changed || !has_live_frame {
                            let span = old.range(zone.zone_index)?.len();
                            let src = &zone.effect.colors;
                            let filled: Vec<[u8; 3]> =
                                (0..span).map(|i| src[i % src.len()]).collect();
                            next.set_direct(zone.zone_index, &filled)?;
                        } else {
                            next.set_direct(
                                zone.zone_index,
                                &old.colors[old.range(zone.zone_index)?],
                            )?;
                        }
                    } else {
                        next.set_direct(zone.zone_index, &old.colors[old.range(zone.zone_index)?])?;
                    }
                } else {
                    next.set_effect(zone.zone_index, &zone.effect)?;
                }
            }
        }
        Ok(ConfiguredRender {
            state: next,
            direct_colors,
        })
    }

    /// None when the device cannot merge, Some(None) when its ports run separately.
    fn port_merge(
        &self,
        config: &RgbAppConfig,
        id: &str,
    ) -> Option<Option<lianli_shared::rgb::RgbPortMerge>> {
        let device = self.wired.get(id)?;
        if device.merged_effect_modes().is_empty() {
            return None;
        }
        let base = port_base(id)?;
        // Quick Sync drives ports separately, so it suspends the merge controller-wide.
        if self
            .sync_active
            .iter()
            .any(|synced| port_base(synced) == Some(base))
        {
            return Some(None);
        }
        Some(
            config
                .devices
                .iter()
                .filter(|saved| port_base(&saved.device_id) == Some(base))
                .filter_map(|saved| {
                    saved
                        .port_merge
                        .as_ref()
                        .filter(|merge| merge.enabled)
                        .map(|merge| (&saved.device_id, merge))
                })
                .min_by_key(|(id, _)| *id)
                .map(|(_, merge)| merge.clone()),
        )
    }

    /// Sends every flag, so clearing a swap reaches the hardware.
    fn apply_directions(&self, device: &lianli_shared::rgb::RgbDeviceConfig) -> anyhow::Result<()> {
        let Some(wired) = self.wired.get(&device.device_id) else {
            return Ok(());
        };
        if wired.supports_port_reverse() {
            wired.set_port_reverse(device.reverse_port)?;
        }
        if wired.supports_direction() {
            let zones = wired.zone_info().len();
            for zone in device
                .zones
                .iter()
                .filter(|zone| usize::from(zone.zone_index) < zones)
            {
                self.set_fan_direction(
                    &device.device_id,
                    zone.zone_index,
                    zone.swap_lr,
                    zone.swap_tb,
                )?;
            }
        }
        Ok(())
    }

    /// The lowest connected port drives the merged group.
    fn is_merge_leader(&self, id: &str) -> bool {
        let Some(base) = port_base(id) else {
            return false;
        };
        self.wired
            .keys()
            .filter(|other| port_base(other) == Some(base))
            .min()
            .is_some_and(|leader| leader == id)
    }
}

pub(super) fn port_base(id: &str) -> Option<&str> {
    id.rsplit_once(":port").map(|(base, _)| base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbDeviceConfig, RgbZoneConfig};
    use std::sync::mpsc;
    use std::time::Duration;

    struct LoopDevice(mpsc::Sender<Vec<Vec<[u8; 3]>>>);

    impl RgbDevice for LoopDevice {
        fn device_name(&self) -> String {
            "loop".into()
        }
        fn supported_modes(&self) -> Vec<RgbMode> {
            vec![RgbMode::Static, RgbMode::Direct]
        }
        fn zone_info(&self) -> Vec<RgbZoneInfo> {
            vec![RgbZoneInfo {
                name: "zone".into(),
                name_ui: None,
                led_count: 1,
            }]
        }
        fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
            Ok(())
        }
        fn software_frame_delivery(&self) -> Option<lianli_devices::traits::RgbFrameDelivery> {
            Some(lianli_devices::traits::RgbFrameDelivery::LoopUpload)
        }
        fn set_software_animation(
            &self,
            frames: &[Vec<[u8; 3]>],
            _: lianli_shared::rgb::RgbPlaybackTiming,
        ) -> anyhow::Result<()> {
            self.0.send(frames.to_vec())?;
            Ok(())
        }
    }

    fn saved_device(id: &str) -> RgbDeviceConfig {
        RgbDeviceConfig {
            device_id: id.into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
            port_merge: None,
            reverse_port: false,
            zones: Vec::new(),
        }
    }

    struct GroupDevice(mpsc::Sender<Vec<RgbEffect>>);

    impl RgbDevice for GroupDevice {
        fn device_name(&self) -> String {
            "group".into()
        }
        fn supported_modes(&self) -> Vec<RgbMode> {
            vec![RgbMode::Static]
        }
        fn zone_info(&self) -> Vec<RgbZoneInfo> {
            vec![]
        }
        fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
            anyhow::bail!("independent fan effects are unsupported")
        }
        fn hardware_regions(&self) -> Vec<lianli_shared::rgb::RgbRegionParameters> {
            vec![lianli_shared::rgb::RgbRegionParameters {
                scope: lianli_shared::rgb::RgbScope::All,
                effects: vec![],
            }]
        }
        fn resolve_group_config(
            &self,
            config: &RgbDeviceConfig,
        ) -> anyhow::Result<Option<Vec<RgbEffect>>> {
            Ok(Some(
                config
                    .regions
                    .iter()
                    .flatten()
                    .map(|region| region.effect.clone())
                    .collect(),
            ))
        }
        fn set_group_effects(&self, effects: &[RgbEffect]) -> anyhow::Result<()> {
            self.0.send(effects.to_vec())?;
            Ok(())
        }
        fn supports_mb_rgb_sync(&self) -> bool {
            true
        }
        fn set_mb_rgb_sync(&self, _: bool) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn returning_to_direct_restores_saved_colors_after_mode_preset_or_zone_changes() {
        for transition in 0..4 {
            let (sender, received) = mpsc::channel();
            let device = Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>;
            let mut controller = RgbController::new(HashMap::from([("live".into(), device)]), None);
            let mut saved = saved_device("live");
            saved.zones = vec![RgbZoneConfig {
                zone_index: 0,
                effect: RgbEffect {
                    mode: RgbMode::Direct,
                    colors: vec![[255, 0, 0]],
                    ..Default::default()
                },
                swap_lr: false,
                swap_tb: false,
            }];
            let original = saved.clone();
            let mut config = RgbAppConfig {
                devices: vec![saved],
                ..Default::default()
            };
            controller.apply_config(&config, &[]);
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                vec![vec![[255, 0, 0]]]
            );
            let presets = vec![RgbPreset {
                name: "blue".into(),
                device_id: "live".into(),
                regions: None,
                zones: vec![RgbPresetZone {
                    zone: 0,
                    colors: vec![],
                    effect: Some(RgbEffect {
                        mode: RgbMode::Static,
                        colors: vec![[0, 0, 255]],
                        ..Default::default()
                    }),
                }],
            }];
            match transition {
                0 => {
                    config.devices[0].zones[0].effect = RgbEffect {
                        mode: RgbMode::Static,
                        colors: vec![[0, 0, 255]],
                        ..Default::default()
                    }
                }
                1 => config.devices[0].zones.clear(),
                2 => config.devices[0].active_preset = Some("blue".into()),
                _ => config.devices.clear(),
            }
            controller.apply_config(&config, &presets);
            controller
                .set_direct_colors("live", 0, &[[0, 0, 255]])
                .unwrap();
            config.devices = vec![original];
            controller.apply_config(&config, &[]);
            assert_eq!(
                controller.get_zone_colors("live", 0).unwrap(),
                vec![[255, 0, 0]],
                "transition {transition}"
            );
            controller.stop();
        }
    }

    #[test]
    fn hardware_groups_apply_presets_once_and_reapply_after_quantity_changes() {
        let (sender, received) = mpsc::channel();
        let mut controller = RgbController::new(
            HashMap::from([(
                "ene:group0".into(),
                Arc::new(GroupDevice(sender)) as Arc<dyn RgbDevice>,
            )]),
            None,
        );
        let mut device = saved_device("ene:group0");
        device.active_preset = Some("preset".into());
        let config = RgbAppConfig {
            devices: vec![device],
            ..Default::default()
        };
        let effects = vec![lianli_shared::rgb::RgbRegionConfig {
            effect: RgbEffect {
                colors: vec![[1, 2, 3], [4, 5, 6]],
                ..Default::default()
            },
            flip: false,
        }];
        let presets = vec![RgbPreset {
            name: "preset".into(),
            device_id: "ene:group0".into(),
            zones: Vec::new(),
            regions: Some(effects.clone()),
        }];
        controller.apply_config(&config, &presets);
        assert_eq!(
            controller.saved_group_effects("ene:group0").unwrap(),
            vec![effects[0].effect.clone()]
        );
        assert_eq!(
            received.try_recv().unwrap(),
            vec![effects[0].effect.clone()]
        );
        controller.apply_config(&config, &presets);
        assert!(received.try_recv().is_err());
        controller.invalidate_device_config("ene:group0");
        controller.apply_config(&config, &presets);
        assert_eq!(
            received.try_recv().unwrap(),
            vec![effects[0].effect.clone()]
        );
        controller.stop();
    }

    /// A Direct zone with colours in config must render those colours on a
    /// fresh start, so a configured per-LED pattern survives a daemon restart.
    ///
    /// Before this, Direct always fell through to the previously rendered
    /// frame, which is all zeros on a fresh start - so the LEDs went black and
    /// the stored colours were never used.
    #[test]
    fn direct_zone_uses_configured_colors_on_fresh_start() {
        let (sender, received) = mpsc::channel();
        let device = Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>;
        let mut controller = RgbController::new(HashMap::from([("live".into(), device)]), None);

        let mut saved = saved_device("live");
        saved.zones = vec![RgbZoneConfig {
            zone_index: 0,
            effect: RgbEffect {
                mode: RgbMode::Direct,
                colors: vec![[9, 8, 7]],
                ..Default::default()
            },
            swap_lr: false,
            swap_tb: false,
        }];
        let config = RgbAppConfig {
            enabled: true,
            devices: vec![saved],
            ..Default::default()
        };

        controller.apply_config(&config, &[]);
        let sent = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            sent,
            vec![vec![[9, 8, 7]]],
            "a fresh start must render the configured Direct colours, not black"
        );
    }

    /// Once a device has a live frame cached, changing a Direct zone's
    /// colours in config and re-applying must still repaint it - saving a
    /// new colour through SetRgbConfig is not "an unrelated config save"
    /// even though both look identical as "a live frame exists".
    #[test]
    fn direct_zone_repaints_when_configured_colors_change_with_a_live_frame_cached() {
        let (sender, received) = mpsc::channel();
        let device = Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>;
        let mut controller = RgbController::new(HashMap::from([("live".into(), device)]), None);

        let mut saved = saved_device("live");
        saved.zones = vec![RgbZoneConfig {
            zone_index: 0,
            effect: RgbEffect {
                mode: RgbMode::Direct,
                colors: vec![[9, 8, 7]],
                ..Default::default()
            },
            swap_lr: false,
            swap_tb: false,
        }];
        let mut config = RgbAppConfig {
            enabled: true,
            devices: vec![saved],
            ..Default::default()
        };

        controller.apply_config(&config, &[]);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[9, 8, 7]]]
        );

        config.devices[0].zones[0].effect.colors = vec![[1, 2, 3]];
        controller.apply_config(&config, &[]);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[1, 2, 3]]],
            "a genuine config colour change must repaint even with a live frame cached"
        );

        // Re-applying the same config again must not repaint a third time.
        controller.apply_config(&config, &[]);
        assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
        controller.stop();
    }

    /// Clearing a zone's configured colours must drop any stale tracking for
    /// it, so a later save that happens to restore the same colour values is
    /// still recognised as a real change rather than matching leftover state
    /// from before the colours were cleared.
    #[test]
    fn direct_zone_clears_stale_tracking_when_colours_are_emptied() {
        let (sender, received) = mpsc::channel();
        let device = Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>;
        let mut controller = RgbController::new(HashMap::from([("live".into(), device)]), None);

        let mut saved = saved_device("live");
        saved.zones = vec![RgbZoneConfig {
            zone_index: 0,
            effect: RgbEffect {
                mode: RgbMode::Direct,
                colors: vec![[9, 8, 7]],
                ..Default::default()
            },
            swap_lr: false,
            swap_tb: false,
        }];
        let mut config = RgbAppConfig {
            enabled: true,
            devices: vec![saved],
            ..Default::default()
        };

        controller.apply_config(&config, &[]);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[9, 8, 7]]]
        );

        // Clear the zone's configured colours, as switching away from a
        // custom pattern would.
        config.devices[0].zones[0].effect.colors = vec![];
        controller.apply_config(&config, &[]);

        // Push an unrelated live frame directly.
        controller
            .set_direct_colors("live", 0, &[[1, 2, 3]])
            .unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[1, 2, 3]]]
        );

        // Restore the original configured colour. Before this fix, the
        // stale tracking entry from the very first apply still said
        // [9,8,7] was already configured, so this would be wrongly treated
        // as unchanged and the live frame would be left in place instead.
        config.devices[0].zones[0].effect.colors = vec![[9, 8, 7]];
        controller.apply_config(&config, &[]);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[9, 8, 7]]],
            "restoring a colour after it was cleared must repaint, not match stale tracking"
        );
        controller.stop();
    }

    #[test]
    fn openrgb_initial_state_resolves_active_presets_without_hardware_writes() {
        let (sender, received) = mpsc::channel();
        let mut controller = RgbController::new(
            HashMap::from([(
                "ene:group0".into(),
                Arc::new(GroupDevice(sender)) as Arc<dyn RgbDevice>,
            )]),
            None,
        );
        let mut device = saved_device("ene:group0");
        device.active_preset = Some("saved".into());
        let effect = RgbEffect {
            mode: RgbMode::Breathing,
            colors: vec![[255, 0, 0]],
            ..Default::default()
        };
        let presets = [RgbPreset {
            name: "saved".into(),
            device_id: device.device_id.clone(),
            zones: vec![],
            regions: Some(vec![lianli_shared::rgb::RgbRegionConfig {
                effect: effect.clone(),
                flip: false,
            }]),
        }];
        let config = RgbAppConfig {
            openrgb_server: true,
            devices: vec![device],
            ..Default::default()
        };
        controller.apply_config(&config, &presets);
        assert_eq!(
            controller.saved_group_effects("ene:group0").unwrap(),
            [effect]
        );
        assert!(received.try_recv().is_err());
        controller.stop();
    }

    #[test]
    fn unrelated_saves_preserve_live_frames_and_direct_colors() {
        for direct in [false, true] {
            let (sender, received) = mpsc::channel();
            let device = Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>;
            let mut controller = RgbController::new(
                HashMap::from([("live".into(), device.clone()), ("other".into(), device)]),
                None,
            );
            let mut config = RgbAppConfig {
                enabled: true,
                devices: vec![saved_device("live")],
                ..Default::default()
            };
            controller.apply_config(&config, &[]);
            received.recv_timeout(Duration::from_secs(1)).unwrap();
            let frames = if direct {
                vec![vec![[12, 34, 56]]]
            } else {
                vec![vec![[255, 0, 0]], vec![[0, 255, 0]]]
            };
            if direct {
                controller.set_direct_colors("live", 0, &frames[0]).unwrap();
            } else {
                controller.set_rgb_frames("live", &frames, 50).unwrap();
            }
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                frames
            );
            controller.apply_config(&config, &[]);
            assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
            config.devices.push(saved_device("other"));
            controller.apply_config(&config, &[]);
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                vec![vec![[0; 3]]]
            );
            assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
            assert_eq!(controller.get_zone_colors("live", 0).unwrap(), frames[0]);
            config.merge_lighting = Some(lianli_shared::rgb::MergeLightingConfig {
                enabled: true,
                kind: lianli_shared::rgb::RgbSyncKind::Matched,
                device_order: vec!["other".into()],
                ..Default::default()
            });
            controller.apply_config(&config, &[]);
            assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
            assert_eq!(controller.get_zone_colors("live", 0).unwrap(), frames[0]);
            config.devices[0].zones.push(RgbZoneConfig {
                zone_index: 0,
                effect: RgbEffect::default(),
                swap_lr: false,
                swap_tb: false,
            });
            controller.apply_config(&config, &[]);
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                vec![vec![[255; 3]]]
            );
            controller.stop();
        }
    }

    #[test]
    fn preset_updates_and_hardware_invalidation_reapply_saved_settings() {
        let (sender, received) = mpsc::channel();
        let mut controller = RgbController::new(
            HashMap::from([(
                "live".into(),
                Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>,
            )]),
            None,
        );
        let mut device = saved_device("live");
        device.active_preset = Some("colors".into());
        let config = RgbAppConfig {
            enabled: true,
            devices: vec![device],
            ..Default::default()
        };
        let mut presets = vec![RgbPreset {
            name: "colors".into(),
            device_id: "live".into(),
            regions: None,
            zones: vec![RgbPresetZone {
                zone: 0,
                colors: vec![[30, 40, 50]],
                effect: None,
            }],
        }];
        controller.apply_config(&config, &presets);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[30, 40, 50]]]
        );
        presets[0].zones[0].colors[0] = [50, 40, 30];
        controller.apply_config(&config, &presets);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[50, 40, 30]]]
        );
        controller.invalidate_hardware_state();
        controller.apply_config(&config, &presets);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![vec![[50, 40, 30]]]
        );
        controller.stop();
    }

    #[test]
    fn unsaved_live_device_survives_configuration_without_device_entries() {
        let (sender, received) = mpsc::channel();
        let mut controller = RgbController::new(
            HashMap::from([(
                "live".into(),
                Arc::new(LoopDevice(sender)) as Arc<dyn RgbDevice>,
            )]),
            None,
        );
        let config = RgbAppConfig {
            enabled: true,
            ..Default::default()
        };
        controller.apply_config(&config, &[]);
        controller
            .set_direct_colors("live", 0, &[[10, 20, 30]])
            .unwrap();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        controller.apply_config(&config, &[]);
        assert_eq!(
            controller.get_zone_colors("live", 0).unwrap(),
            [[10, 20, 30]]
        );
        assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
        controller.stop();
    }

    #[test]
    fn detached_fan_settings_are_preserved_but_not_rendered() {
        let mut controller = RgbController::new(HashMap::new(), None);
        controller.wireless_state.insert(
            "tl".into(),
            WirelessDevice {
                mac: [0; 6],
                fan_count: 3,
                fan_type: WirelessFanType::Tlv2Led,
                right_attach: false,
            },
        );
        let config = RgbDeviceConfig {
            device_id: "tl".into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
            port_merge: None,
            reverse_port: false,
            zones: (0..4)
                .map(|zone_index| RgbZoneConfig {
                    zone_index,
                    swap_lr: false,
                    swap_tb: false,
                    effect: RgbEffect {
                        mode: if zone_index == 3 {
                            RgbMode::Rainbow
                        } else {
                            RgbMode::Direct
                        },
                        ..Default::default()
                    },
                })
                .collect(),
        };
        let state = controller.configured_render(&config, &[]).unwrap().state;
        assert_eq!(state.counts, [26; 3]);
        assert_eq!(state.colors.len(), 78);
        assert!(state.regions.is_none());
        assert_eq!(config.zones.len(), 4);
        assert_eq!(config.zones[3].effect.mode, RgbMode::Rainbow);
    }
}
