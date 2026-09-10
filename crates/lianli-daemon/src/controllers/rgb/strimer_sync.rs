use super::*;
use anyhow::{ensure, Result};
use lianli_media::rgb::{strimer, Animation};
use lianli_shared::rgb::RgbRegionConfig;

impl RgbController {
    pub(super) fn is_short_strimer(&self, id: &str) -> bool {
        self.wireless_state
            .get(id)
            .is_some_and(|device| matches!(device.fan_type, WirelessFanType::Strimer(2 | 4)))
    }

    pub(super) fn retime_strimer(
        &self,
        id: &str,
        regions: &[RgbRegionConfig],
        animation: &mut Animation,
    ) -> Result<()> {
        if !self.is_short_strimer(id) {
            return Ok(());
        }
        let Some(wireless) = &self.wireless else {
            return Ok(());
        };
        let Some(reference) = wireless
            .devices()
            .into_iter()
            .find(|device| matches!(device.fan_type, WirelessFanType::Strimer(1 | 3)))
        else {
            return Ok(());
        };
        let reference_id = format!("wireless:{}", reference.mac_str());
        let Some(reference_regions) = self
            .rendered
            .get(&reference_id)
            .and_then(|state| state.regions.as_ref())
        else {
            return Ok(());
        };
        let Some(profile) = self.regional_profile(&reference_id) else {
            return Ok(());
        };
        let target_key = strimer::sync_key(regions, animation.frames[0].len())?;
        let reference_key = strimer::sync_key(reference_regions, usize::from(profile.led_count))?;
        if target_key == reference_key {
            let reference_animation =
                lianli_media::rgb::family::render(profile, reference_regions)?;
            animation.interval_hundredths = matched_interval(
                reference_animation.interval_hundredths,
                reference_animation.frames.len(),
                animation.frames.len(),
            )?;
        }
        Ok(())
    }
}

pub(super) fn matched_interval(
    reference_interval: u32,
    reference_frames: usize,
    target_frames: usize,
) -> Result<u32> {
    ensure!(
        reference_frames > 0 && target_frames > 0,
        "cannot synchronize an empty RGB animation"
    );
    let ticks =
        f64::from(reference_interval) / 100.0 * reference_frames as f64 / target_frames as f64;
    let hundredths = (ticks * 100.0) as u32;
    ensure!(
        (100..=6_553_599).contains(&hundredths),
        "synchronized RGB interval exceeds the RF protocol"
    );
    Ok(hundredths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_strimer_matches_the_long_strimer_loop_duration() {
        assert_eq!(matched_interval(11_000, 348, 264).unwrap(), 14_500);
        assert_eq!(matched_interval(11_000, 174, 264).unwrap(), 7_250);
        assert_eq!(matched_interval(2_200, 29, 22).unwrap(), 2_900);
        assert_eq!(matched_interval(2_200, 66, 29).unwrap(), 5_006);
        assert!(matched_interval(2_200, 0, 29).is_err());
        assert!(matched_interval(2_200, 29, 0).is_err());
        assert!(matched_interval(6_553_599, 348, 1).is_err());
    }
}
