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
        self.clear_pending();
        let ids: std::collections::HashSet<_> =
            plan.iter().map(|item| item.id().to_owned()).collect();
        let clocks = ids
            .iter()
            .filter_map(|id| self.wired.get(id).cloned())
            .collect();
        self.sync_clock.configure(clocks, self.wireless.clone());
        self.sync_active = ids;
        for item in plan {
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
                    if device.supports_mb_rgb_sync() {
                        device.set_mb_rgb_sync(false)?;
                    }
                    device.set_all_effects(&effect)?;
                    self.mb_sync_state.insert(id, false);
                }
            }
        }
        self.sync_signature = signature;
        for id in previous {
            if self.sync_active.contains(&id) || config.devices.iter().any(|d| d.device_id == id) {
                continue;
            }
            if self.software_controlled(&id) {
                let mut restore = self.render_state(&id)?;
                self.submit_render(&id, &mut restore)?;
            } else if let Some(device) = self.wired.get(&id) {
                device.set_all_effects(&RgbEffect::default())?;
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
