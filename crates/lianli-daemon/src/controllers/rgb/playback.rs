use super::*;

impl RgbController {
    pub(super) fn submit_render(
        &mut self,
        id: &str,
        state: &mut RenderState,
    ) -> anyhow::Result<()> {
        if let Some(device) = self.wired.get(id) {
            if let (Some(settings), Some(profile)) = (&state.regions, self.regional_profile(id)) {
                let animation = lianli_media::rgb::family::render(profile, settings)?;
                state.colors.clone_from(&animation.frames[0]);
                let timing = animation.timing();
                self.wired_renderer.submit_animation(
                    id,
                    device.clone(),
                    animation.frames,
                    timing,
                )?;
                self.mb_sync_state.insert(id.to_owned(), false);
                return Ok(());
            }
            self.wired_renderer
                .submit(id, device.clone(), state.frames(), FRAME_INTERVAL_MS)?;
        } else if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id))
        {
            let upload = if let (Some(regions), Some(profile)) =
                (&state.regions, self.regional_profile(id))
            {
                let mut animation = lianli_media::rgb::family::render(profile, regions)?;
                self.retime_strimer(id, regions, &mut animation)?;
                state.colors.clone_from(&animation.frames[0]);
                wireless.prepare_rgb_animation(
                    &device.mac,
                    &animation.frames,
                    animation.timing(),
                )?
            } else {
                wireless.prepare_rgb_upload(&device.mac, &state.frames(), FRAME_INTERVAL_MS)?
            };
            let upload = Arc::new(upload);
            self.upload_worker.submit(
                wireless.clone(),
                device.mac,
                Command::Upload(upload.clone()),
            )?;
            self.uploads.insert(id.to_owned(), upload);
        } else {
            anyhow::bail!("RGB device not found: {id}");
        }
        self.mb_sync_state.insert(id.to_owned(), false);
        Ok(())
    }

    pub(super) fn apply_render(&mut self, id: &str, mut state: RenderState) -> anyhow::Result<()> {
        if let Some(applied) = self.applied.get(id).filter(|old| old.same_render(&state)) {
            if state.regions.is_some() {
                state.colors.clone_from(&applied.colors);
            }
        } else {
            self.submit_render(id, &mut state)?;
            self.applied.insert(id.to_owned(), state.clone());
        }
        self.rendered.insert(id.to_owned(), state);
        Ok(())
    }
}
