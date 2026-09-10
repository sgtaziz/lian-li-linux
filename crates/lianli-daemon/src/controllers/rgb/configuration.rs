use super::*;
use anyhow::Context;

impl RgbController {
    pub fn validate_config(&self, config: &RgbAppConfig) -> anyhow::Result<()> {
        if !config.enabled || config.openrgb_server {
            return Ok(());
        }
        for device in &config.devices {
            if device.mb_rgb_sync || !self.software_controlled(&device.device_id) {
                continue;
            }
            let state = self.configured_render(device, &self.presets)?;
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
        self.config = Some(config.clone());
        self.presets = presets.to_vec();
        self.openrgb_server_enabled = config.openrgb_server;
        if !config.enabled || self.is_openrgb_controlled() {
            self.clear_pending();
            return;
        }
        if self.thermal_override_active() {
            return;
        }

        let removed: Vec<_> = self
            .rendered
            .keys()
            .filter(|id| !config.devices.iter().any(|d| &d.device_id == *id))
            .cloned()
            .collect();
        if removed
            .iter()
            .any(|id| self.wireless_state.contains_key(id))
        {
            self.upload_worker.clear();
            if let Some(wireless) = &self.wireless {
                wireless.clear_rgb_targets();
            }
            self.applied
                .retain(|id, _| !self.wireless_state.contains_key(id));
        }
        for id in removed {
            self.wired_renderer.remove(&id);
            self.rendered.remove(&id);
            self.applied.remove(&id);
            self.uploads.remove(&id);
        }

        let mut ordered: Vec<_> = config.devices.iter().collect();
        ordered.sort_by_key(|device| self.is_short_strimer(&device.device_id));
        for device in ordered {
            let result = (|| -> anyhow::Result<()> {
                if device.mb_rgb_sync {
                    return self.set_mb_rgb_sync(&device.device_id, true);
                }
                if self.software_controlled(&device.device_id) {
                    let next = self.configured_render(device, presets)?;
                    self.apply_render(&device.device_id, next)?;
                } else {
                    for zone in &device.zones {
                        self.set_effect(&device.device_id, zone.zone_index, &zone.effect)?;
                        if zone.swap_lr || zone.swap_tb {
                            self.set_fan_direction(
                                &device.device_id,
                                zone.zone_index,
                                zone.swap_lr,
                                zone.swap_tb,
                            )?;
                        }
                    }
                }
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

    fn configured_render(
        &self,
        device: &lianli_shared::rgb::RgbDeviceConfig,
        presets: &[RgbPreset],
    ) -> anyhow::Result<RenderState> {
        let old = self.render_state(&device.device_id)?;
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
        if next.regions.is_none() {
            for zone in &effective.zones {
                if zone.effect.mode == RgbMode::Direct {
                    let colors = preset.and_then(|preset| {
                        preset
                            .zones
                            .iter()
                            .find(|entry| entry.zone == zone.zone_index && !entry.colors.is_empty())
                    });
                    if let Some(colors) = colors {
                        next.set_direct(zone.zone_index, &colors.colors)?;
                    } else {
                        next.set_direct(zone.zone_index, &old.colors[old.range(zone.zone_index)?])?;
                    }
                } else {
                    next.set_effect(zone.zone_index, &zone.effect)?;
                }
            }
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbDeviceConfig, RgbZoneConfig};

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
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
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
        let state = controller.configured_render(&config, &[]).unwrap();
        assert_eq!(state.counts, [26; 3]);
        assert_eq!(state.colors.len(), 78);
        assert!(state.regions.is_none());
        assert_eq!(config.zones.len(), 4);
        assert_eq!(config.zones[3].effect.mode, RgbMode::Rainbow);
    }
}
