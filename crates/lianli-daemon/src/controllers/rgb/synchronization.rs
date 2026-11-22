use super::*;
use anyhow::Result;
use sync_plan::PreparedSync;

impl RgbController {
    pub(super) fn apply_sync(&mut self, config: &RgbAppConfig) -> Result<()> {
        let signature = self.sync_signature(config)?;
        if signature == self.sync_signature && (signature.is_some() || self.sync_active.is_empty())
        {
            return Ok(());
        }
        let plan = self.prepare_sync(config)?;
        let previous = self.sync_active.clone();
        let ids: std::collections::HashSet<_> =
            plan.iter().map(|item| item.id().to_owned()).collect();
        for id in previous.union(&ids) {
            self.clear_device_pending(id);
        }
        self.sync_signature = None;
        // A port's motherboard-sync reset can clear every port on its controller.
        let mut reset_failures = std::collections::HashSet::new();
        for item in &plan {
            if let PreparedSync::Hardware { id, device, .. } = item {
                if device.supports_mb_rgb_sync() {
                    let base = id
                        .rsplit_once(":port")
                        .map_or(id.as_str(), |(base, _)| base);
                    self.configured.retain(|other, _| {
                        other != base
                            && !other
                                .strip_prefix(base)
                                .is_some_and(|suffix| suffix.starts_with(":port"))
                    });
                    if let Err(error) = device.set_mb_rgb_sync(false) {
                        warn!("Failed to leave motherboard RGB sync for {id}: {error:#}");
                        reset_failures.insert(id.clone());
                    }
                }
            }
        }
        for item in &plan {
            let id = item.id();
            let Some(base) = super::configuration::port_base(id) else {
                continue;
            };
            let Some(device) = self
                .wired
                .get(id)
                .filter(|device| !device.merged_effect_modes().is_empty())
                .cloned()
            else {
                continue;
            };
            self.configured
                .retain(|other, _| super::configuration::port_base(other) != Some(base));
            if let Err(error) = device.apply_port_merge(None) {
                warn!("Failed to suspend the port merge for {id}: {error:#}");
                reset_failures.insert(id.to_owned());
            }
        }
        let clocks = ids
            .iter()
            .filter_map(|id| self.wired.get(id).cloned())
            .collect();
        self.sync_clock.configure(clocks, self.wireless.clone());
        self.sync_active = ids;
        for id in previous {
            if self.sync_active.contains(&id) || config.devices.iter().any(|d| d.device_id == id) {
                continue;
            }
            let restored = if self.software_controlled(&id) {
                self.render_state(&id)
                    .and_then(|mut state| self.submit_render(&id, &mut state))
            } else if let Some(device) = self.wired.get(&id) {
                device.set_all_effects(&RgbEffect::default())
            } else {
                Ok(())
            };
            if let Err(error) = restored {
                warn!("Failed to restore RGB for {id}: {error:#}");
            }
        }
        let mut failure = None;
        for item in plan {
            let id = item.id().to_owned();
            if reset_failures.contains(&id) {
                failure = Some(anyhow::anyhow!("failed to prepare {id} for RGB sync"));
                continue;
            }
            if let Err(error) = self.submit_sync(item) {
                warn!("Failed to submit RGB sync for {id}: {error:#}");
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.sync_signature = signature;
        Ok(())
    }

    fn submit_sync(&mut self, item: PreparedSync) -> Result<()> {
        match item {
            PreparedSync::Wired {
                id,
                device,
                animation,
            } => {
                let timing = animation.timing();
                self.wired_renderer.submit_timed(
                    &id,
                    device,
                    animation.frames,
                    timing,
                    Some(self.sync_clock.clock.clone()),
                )?;
                self.mb_sync_state.insert(id, false);
            }
            PreparedSync::Wireless { id, mac, upload } => {
                let wireless = self
                    .wireless
                    .as_ref()
                    .expect("prepared wireless RGB controller");
                self.upload_worker.submit(
                    wireless.clone(),
                    mac,
                    Command::Upload(upload.clone()),
                )?;
                self.uploads.insert(id.clone(), upload);
                self.mb_sync_state.insert(id, false);
            }
            PreparedSync::Hardware { id, device, effect } => {
                device.set_all_effects(&effect)?;
                self.mb_sync_state.insert(id, false);
            }
        }
        Ok(())
    }

    pub(super) fn ensure_individual_control(&self, id: &str) -> Result<()> {
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.sync_active.contains(id),
            "disable RGB sync for {id} before changing its individual lighting"
        );
        Ok(())
    }
}
