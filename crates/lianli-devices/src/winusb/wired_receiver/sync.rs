use super::*;

impl WiredReceiverController {
    pub(super) fn validate_sync_frames(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        let profile = self
            .software_render_profile()
            .context("receiver RGB layout unavailable")?;
        let leds = usize::from(profile.sync_frame_led_count());
        let count = checked_frame_count(frames.len())?;
        anyhow::ensure!(
            frames.iter().all(|frame| frame.len() == leds),
            "sync RGB frame does not match receiver layout"
        );
        anyhow::ensure!(
            (100..=6_553_599).contains(&timing.interval_hundredths),
            "sync RGB interval out of range"
        );
        anyhow::ensure!(
            timing.secondary_frame_count == 0
                && timing.secondary_interval_ticks == 0
                && !timing.outer_longest,
            "sync RGB requires a single timeline"
        );
        if self.params.compresses_rgb {
            let compressed = crate::tinyuz::compress(&Self::rgb_frames_bytes(frames))?;
            anyhow::ensure!(
                compressed.len() <= 12_288,
                "sync RGB animation exceeds receiver memory"
            );
            rgb_flash_header(compressed.len(), leds, 0, count, timing)?;
        }
        Ok(())
    }

    pub(super) fn send_sync_animation(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        if self.rf_owned() {
            return Ok(());
        }
        self.validate_sync_frames(frames, timing)?;
        let raw = Self::rgb_frames_bytes(frames);
        if self.params.compresses_rgb {
            self.send_rgb_flash_save(
                &raw,
                frames[0].len(),
                self.effect_nonce.fetch_add(1, Ordering::Relaxed).max(1),
                checked_frame_count(frames.len())?,
                timing,
            )
        } else {
            anyhow::ensure!(frames.len() == 1, "streaming sync accepts one frame");
            self.send_rgb_stream(&raw)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WiredReceiverController;

    #[test]
    fn sync_frames_keep_projected_right_attach_fan_order() {
        let frame: Vec<_> = std::iter::repeat_n([1, 2, 3], 44)
            .chain(std::iter::repeat_n([4, 5, 6], 44))
            .collect();
        let raw = WiredReceiverController::rgb_frames_bytes(&[frame]);

        assert_eq!(&raw[0..3], &[1, 2, 3]);
        assert_eq!(&raw[129..135], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(&raw[261..264], &[4, 5, 6]);
    }
}
