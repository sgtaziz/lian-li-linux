use super::*;

impl RgbController {
    pub fn set_effect(&mut self, id: &str, zone: u8, effect: &RgbEffect) -> anyhow::Result<()> {
        self.ensure_individual_control(id)?;
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.software_controlled(id) {
            let mut state = self.render_state(id)?;
            if let Some(profile) = self
                .regional_profile(id)
                .filter(|_| effect.mode != RgbMode::Direct)
            {
                anyhow::ensure!(
                    zone == 0,
                    "animation modes apply to the device group, not individual fans"
                );
                let mut effect = effect.clone();
                if profile.family == lianli_shared::rgb::RgbRenderFamily::HydroShiftII
                    && effect.scope == lianli_shared::rgb::RgbScope::All
                {
                    effect.scope = lianli_shared::rgb::RgbScope::Pump;
                }
                let mut regions = state.regions.take().unwrap_or_default();
                let flip = regions.first().is_some_and(|region| region.flip);
                if effect.scope == lianli_shared::rgb::RgbScope::All {
                    regions.clear();
                } else if let Some(all) = regions
                    .iter()
                    .find(|r| r.effect.scope == lianli_shared::rgb::RgbScope::All)
                    .cloned()
                {
                    regions = regions::expand_all(profile, &all);
                }
                regions.retain(|region| region.effect.scope != effect.scope);
                regions.push(lianli_shared::rgb::RgbRegionConfig { effect, flip });
                state.regions = Some(regions);
                return self.apply_render(id, state);
            }
            state.set_effect(zone, effect)?;
            self.apply_render(id, state)?;
            return Ok(());
        }
        if let Some(device) = self.wired.get(id) {
            anyhow::ensure!(
                device.supported_modes().contains(&effect.mode),
                "RGB effect {:?} is not supported by {id}",
                effect.mode
            );
            device.set_zone_effect(zone, effect)?;
            return Ok(());
        }
        anyhow::bail!("RGB device is unavailable or has an unsupported layout: {id}")
    }

    pub fn cache_direct_batch(&mut self, updates: &HashMap<String, HashMap<u8, Vec<[u8; 3]>>>) {
        for (id, zones) in updates {
            for (&zone, colors) in zones {
                self.last_direct.insert((id.clone(), zone), colors.clone());
            }
        }
    }

    pub fn set_direct_colors(
        &mut self,
        id: &str,
        zone: u8,
        colors: &[[u8; 3]],
    ) -> anyhow::Result<()> {
        self.ensure_individual_control(id)?;
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.software_controlled(id) {
            let mut state = self.render_state(id)?;
            state.set_direct(zone, colors)?;
            self.apply_render(id, state)?;
        } else if let Some(device) = self.wired.get(id) {
            device.set_direct_colors(zone, colors)?;
        } else {
            anyhow::bail!("RGB device is unavailable or has an unsupported layout: {id}");
        }
        self.last_direct
            .insert((id.to_owned(), zone), colors.to_vec());
        Ok(())
    }

    pub fn apply_wireless_batch(
        &mut self,
        id: &str,
        zones: &HashMap<u8, Vec<[u8; 3]>>,
    ) -> anyhow::Result<()> {
        self.ensure_individual_control(id)?;
        anyhow::ensure!(
            self.software_controlled(id),
            "device {id} does not support software RGB"
        );
        let mut state = self.render_state(id)?;
        for (&zone, colors) in zones {
            state.set_direct(zone, colors)?;
        }
        self.apply_render(id, state)?;
        for (&zone, colors) in zones {
            self.last_direct
                .insert((id.to_owned(), zone), colors.clone());
        }
        Ok(())
    }

    pub fn resync_wireless_direct_colors(&mut self) {
        self.resync_wireless_effects();
    }

    pub fn resync_wireless_effects(&mut self) {
        if self.config.as_ref().is_some_and(|config| !config.enabled) {
            return;
        }
        if let Some(wireless) = &self.wireless {
            for (id, upload) in &self.uploads {
                if self.mb_sync_state.get(id) == Some(&true) {
                    continue;
                }
                if let Some(device) = self.wireless_state.get(id) {
                    let already_applied =
                        wireless
                            .device_by_mac(&device.mac)
                            .is_some_and(|discovered| {
                                !discovered.is_sync_mb_light
                                    && discovered.effect_index == upload.effect_index()
                            });
                    if already_applied {
                        continue;
                    }
                    if let Err(error) = self.upload_worker.submit(
                        wireless.clone(),
                        device.mac,
                        Command::Upload(upload.clone()),
                    ) {
                        debug!("RGB resync failed for {id}: {error}");
                    }
                }
            }
        }
    }

    pub fn set_rgb_frames(
        &mut self,
        id: &str,
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
    ) -> anyhow::Result<()> {
        self.ensure_individual_control(id)?;
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        anyhow::ensure!(
            self.software_controlled(id),
            "device {id} does not support software RGB"
        );
        anyhow::ensure!(
            interval_ms >= FRAME_INTERVAL_MS,
            "RGB animation is limited to 20 frames per second"
        );
        let mut state = self.render_state(id)?;
        anyhow::ensure!(
            !frames.is_empty() && frames.len() <= 2048,
            "invalid RGB frame count"
        );
        anyhow::ensure!(
            frames.iter().all(|frame| frame.len() == state.colors.len()),
            "RGB frame does not match device layout"
        );
        if let Some(device) = self.wired.get(id) {
            self.wired_renderer
                .submit(id, device.clone(), frames.to_vec(), interval_ms)?;
        } else if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id))
        {
            let upload = Arc::new(wireless.prepare_rgb_upload(&device.mac, frames, interval_ms)?);
            self.upload_worker.submit(
                wireless.clone(),
                device.mac,
                Command::Upload(upload.clone()),
            )?;
            self.uploads.insert(id.to_owned(), upload);
        } else {
            anyhow::bail!("RGB device not found: {id}");
        }
        state.colors.copy_from_slice(&frames[0]);
        state.effects.fill(None);
        state.regions = None;
        self.applied.remove(id);
        self.rendered.insert(id.to_owned(), state);
        self.mb_sync_state.insert(id.to_owned(), false);
        Ok(())
    }

    pub fn set_mb_rgb_sync(&mut self, id: &str, enabled: bool) -> anyhow::Result<()> {
        if enabled {
            self.ensure_individual_control(id)?;
        }
        anyhow::ensure!(
            !enabled || self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.mb_sync_state.get(id) == Some(&enabled) {
            return Ok(());
        }
        if let Some(device) = self.wired.get(id) {
            anyhow::ensure!(
                device.supports_mb_rgb_sync(),
                "Device {id} does not support MB RGB sync"
            );
            self.wired_renderer.remove(id);
            device.set_mb_rgb_sync(enabled)?;
        } else if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id))
        {
            anyhow::ensure!(
                device.fan_type.supports_mb_rgb_sync(),
                "Device {id} does not support MB RGB sync"
            );
            self.upload_worker.submit(
                wireless.clone(),
                device.mac,
                Command::MotherboardSync(enabled),
            )?;
        } else {
            anyhow::bail!("RGB device not found: {id}");
        }
        self.applied.remove(id);
        self.mb_sync_state.insert(id.to_owned(), enabled);
        Ok(())
    }

    pub fn set_fan_direction(
        &self,
        id: &str,
        zone: u8,
        swap_lr: bool,
        swap_tb: bool,
    ) -> anyhow::Result<()> {
        self.ensure_individual_control(id)?;
        let device = self
            .wired
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("RGB device not found: {id}"))?;
        anyhow::ensure!(
            device.supports_direction(),
            "Device {id} does not support fan direction"
        );
        device.set_fan_direction(zone, swap_lr, swap_tb)
    }
}
